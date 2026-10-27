#!/usr/bin/env python3
"""Hash-verified Windows executor for windows-dev.py; no machine-specific defaults."""
import base64
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import time
import zipfile


def digest(path):
    with Path(path).open('rb') as stream:
        value = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
        return value.hexdigest()


def plain(path):
    path = Path(path)
    for entry in [path, *path.parents]:
        if entry.exists() and (entry.is_symlink() or
                getattr(entry.lstat(), 'st_file_attributes', 0) & 1024):
            raise ValueError('Reparse paths are not development-runner storage')
    return path


def identifier(value, pattern=r'[a-f0-9]{32}'):
    if not isinstance(value, str) or not re.fullmatch(pattern, value):
        raise ValueError('Invalid runner identifier')
    return value


def pe(path, installer=False):
    with path.open('rb') as stream:
        header = stream.read(64)
        if len(header) != 64 or header[:2] != b'MZ':
            raise ValueError('Expected a native executable')
        stream.seek(struct.unpack_from('<I', header, 60)[0])
        expected = b'PE\0\0\x4c\x01' if installer else b'PE\0\0\x64\x86'
        if stream.read(6) != expected:
            raise ValueError('Expected an NSIS x86 stub' if installer else 'Expected an x64 PE artifact')


def manifest(directory):
    plain(directory)
    value = json.loads((directory / 'manifest.json').read_text(encoding='utf-8'))
    identifier(value['source_revision'], r'[a-f0-9]{40}')
    if value['target'] != 'x86_64-pc-windows-msvc' or not value['files']:
        raise ValueError('Expected explicit Windows MSVC artifacts')
    for name, expected in value['files'].items():
        identifier(name, r'[A-Za-z0-9_-]+\.exe')
        file = plain(directory / name)
        if file.stat().st_nlink != 1 or file.stat().st_size != expected['bytes'] or digest(file) != expected['sha256']:
            raise ValueError('Artifact identity differs from its manifest')
        installer = name == 'installer.exe' and value.get('installer_format') == 'nsis'
        if name == 'installer.exe' and not installer:
            raise ValueError('Installer requires an explicit NSIS bundle declaration')
        pe(file, installer=installer)
    return value


def stage(root, archive, checksum):
    archive = plain(archive)
    if digest(archive) != checksum:
        raise ValueError('Transferred archive hash differs')
    with zipfile.ZipFile(archive) as source:
        names = source.namelist()
        if len(names) != len(set(names)) or 'manifest.json' not in names:
            raise ValueError('Duplicate or absent manifest')
        for item in source.infolist():
            if not re.fullmatch(r'(manifest\.json|[A-Za-z0-9_-]+\.exe)', item.filename):
                raise ValueError('Only flat executable bundles are accepted')
            if (item.external_attr >> 16) & 0o170000 == 0o120000 or item.file_size > 512 * 1024 * 1024:
                raise ValueError('Unsupported archive entry')
        if sum(item.file_size for item in source.infolist()) > 2 * 1024**3:
            raise ValueError('Bundle exceeds development artifact bound')
        value = json.loads(source.read('manifest.json'))
        identifier(value['source_revision'], r'[a-f0-9]{40}')
        if set(names) != {'manifest.json', *value['files']}:
            raise ValueError('Unmanifested archive contents')
        name = value['source_revision'][:12] + '-' + checksum[:12]
        target = plain(root / 'bundles' / name)
        if not target.exists():
            target.mkdir(parents=True)
            try:
                source.extractall(target)
                manifest(target)
            except Exception:
                # Keep failed bytes for diagnosis, never reuse a partial stage.
                (target / 'invalid').touch()
                raise
        if (target / 'invalid').exists():
            raise ValueError('Previous stage failed; preserve it and use a corrected bundle')
        verified = manifest(target)
    return dict(bundle=name, source_revision=verified['source_revision'], files=list(verified['files']))


def ps(script):
    result = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-EncodedCommand',
                             base64.b64encode(script.encode('utf-16le')).decode()],
                            capture_output=True, text=True, errors='replace', timeout=30,
                            creationflags=0x08000000 if os.name == 'nt' else 0)
    if result.returncode:
        raise RuntimeError('Scheduled task operation failed: ' + result.stderr[-1800:])
    return result.stdout.strip()


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def task_name(run_id):
    return 'HiRoute-Dev-' + identifier(run_id)


def task_state(run_id):
    value = ps("$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"
               '$t=Get-ScheduledTask -TaskName ' + literal(task_name(run_id)) +
               ' -ErrorAction SilentlyContinue; if($t){$i=$t|Get-ScheduledTaskInfo;'
               '@{state=[string]$t.State;result=$i.LastTaskResult}|ConvertTo-Json -Compress}'
               'else{\'{"state":"absent"}\'}')
    return json.loads(value)


def directory(root, run_id):
    return plain(root / 'runs' / identifier(run_id))


def worker_image(request):
    path = Path(request['config']['python'])
    return path.with_name('pythonw.exe') if request.get('operation') == 'desktop' else path


def nsis_uninstall_command(executable, programs):
    # NSIS documents an unquoted, last _?= tail to suppress its detached copy.
    # CreateProcess receives this directly; there is no command shell involved.
    value = str(programs)
    if any(c in value for c in '\x00\r\n"'):
        raise ValueError('Invalid native installation directory')
    return subprocess.list2cmdline([str(executable), '/S']) + ' _?=' + value


def private_runs(root, user):
    path = plain(root / 'runs')
    # Raw native output stays under a private runner-owned parent. Never repair
    # permissions on an existing directory with an unrelated access policy.
    script = "$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"
    script += '$path=' + literal(path) + ';$account=' + literal(user) + ';'
    script += r'''
$sid=[Security.Principal.SecurityIdentifier]
$current=[Security.Principal.WindowsIdentity]::GetCurrent().User
$target=([Security.Principal.NTAccount]::new($account)).Translate($sid)
$allowed=@($current.Value,$target.Value,'S-1-5-18','S-1-5-32-544')|Select-Object -Unique
if(-not (Test-Path -LiteralPath $path)) {
 New-Item -ItemType Directory -Path $path | Out-Null
 $acl=[Security.AccessControl.DirectorySecurity]::new()
 $acl.SetOwner($current);$acl.SetAccessRuleProtection($true,$false)
 foreach($id in $allowed){
  $rule=[Security.AccessControl.FileSystemAccessRule]::new(
   [Security.Principal.SecurityIdentifier]::new($id),'FullControl','ContainerInherit,ObjectInherit','None','Allow')
  $acl.AddAccessRule($rule)
 }
 Set-Acl -LiteralPath $path -AclObject $acl
}
$acl=Get-Acl -LiteralPath $path
if(-not $acl.AreAccessRulesProtected){throw 'Runner evidence directory inherits foreign access'}
foreach($rule in $acl.GetAccessRules($true,$true,$sid)){
 if($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $allowed){
  throw 'Runner evidence directory admits an unrelated identity'
 }
}
$inherit=[Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit'
$full=[Security.AccessControl.FileSystemRights]::FullControl
$ownerRules=@($acl.GetAccessRules($true,$false,$sid)|Where-Object {
 $_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Value -eq $target.Value -and
 ($_.FileSystemRights -band $full) -eq $full -and
 ($_.InheritanceFlags -band $inherit) -eq $inherit -and
 $_.PropagationFlags -eq [Security.AccessControl.PropagationFlags]::None
})
if($ownerRules.Count -eq 0){
 throw 'Runner evidence directory does not grant the configured account inheritable access. Use a separate HOME-configured root for this account.'
}
'''
    ps(script)


def status(root, run_id):
    run = directory(root, run_id)
    task = task_state(run_id)
    result = run / 'result.json'
    if result.exists():
        value = json.loads(result.read_text(encoding='utf-8'))
        return dict(run_id=run_id, state='passed' if value['passed'] else 'failed',
                    task=task, result=value)
    return dict(run_id=run_id, state='running' if task['state'] == 'Running' else 'failed',
                task=task, reason='No terminal result yet; task exit alone cannot prove a test')


def start(root, request):
    config = request['config']
    run_id = identifier(request['run_id'])
    if not 1 <= request['timeout'] <= 1800:
        raise ValueError('Whole-case watchdog must be between 1 and 1800 seconds')
    for field in ('harness', 'product'):
        identifier(request[field], r'[a-f0-9]{12}-[a-f0-9]{12}')
        manifest(root / 'bundles' / request[field])
    identifier(request['binary'], r'[A-Za-z0-9_-]+\.exe')
    identifier(request['case'], r'[A-Za-z0-9_:]+')
    worker_python = plain(worker_image(request))
    if request.get('operation') == 'desktop':
        pe(worker_python)
    private_runs(root, config['user'])
    lock = root / 'active.json'
    # A disconnected controller must not permit concurrent mutation of the same
    # native account. Explicit finish retires only a verified stopped task.
    with lock.open('x', encoding='utf-8') as stream:
        json.dump(dict(run_id=run_id), stream)
    run = directory(root, run_id)
    run.mkdir(parents=True)
    (run / 'request.json').write_text(json.dumps(request), encoding='utf-8')
    arguments = subprocess.list2cmdline([request['driver_path'], '--worker', str(run / 'request.json')])
    script = "$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"
    script += '$name=' + literal(task_name(run_id)) + ';'
    script += "if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Task already exists'};"
    script += '$a=New-ScheduledTaskAction -Execute ' + literal(worker_python) + ' -Argument ' + literal(arguments) + ';'
    script += '$p=New-ScheduledTaskPrincipal -UserId ' + literal(config['user']) + ' -LogonType Interactive -RunLevel Limited;'
    script += '$s=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds ' + str(request['timeout'] + 90) + ') -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries;'
    script += 'Register-ScheduledTask -TaskName $name -Action $a -Principal $p -Settings $s|Out-Null;Start-ScheduledTask -TaskName $name'
    ps(script)
    return dict(run_id=run_id, state='submitted', task=task_name(run_id))


def profile_helper(request):
    import importlib.util
    path = plain(request['profile_helper_path'])
    if digest(path) != request['profile_helper_sha256']:
        raise ValueError('Profile helper differs from the committed request')
    spec = importlib.util.spec_from_file_location('windows_dev_profile', path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return helper


def desktop_helper(request):
    import importlib.util
    path = plain(request['desktop_helper_path'])
    if digest(path) != request['desktop_helper_sha256']:
        raise ValueError('Desktop helper differs from the committed request')
    spec = importlib.util.spec_from_file_location('windows_desktop_session', path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return helper


def profile_activity(config, run_id):
    native_home = Path.home()
    roots = [native_home / '.local/share/hiroute', Path(os.environ['LOCALAPPDATA']) / 'HiRoute/cli',
             Path(os.environ['LOCALAPPDATA']) / 'Programs/HiRoute']
    script = "$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"
    script += '$roots=@(' + ','.join(literal(p) for p in roots) + ');'
    script += '$session=' + str(config['session_id']) + ';$ownTask=' + literal(task_name(run_id)) + ';'
    script += r"""
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
function InProductPath($value) {
 if(-not $value){return $false}
 foreach($root in $roots){if($value.StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)){return $true}}
 return $false
}
$processes=@(Get-CimInstance Win32_Process | Where-Object {
 (InProductPath $_.ExecutablePath) -or ($_.SessionId -eq $session -and $_.Name -in @('hiroute.exe','hirouted.exe','hiroute-shell.exe','hiroute-preflight.exe','hiroute-uninstall-helper.exe','installer.exe','cliproxyapi.exe','windows_headless.exe','windows_desktop.exe'))
} | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,@{Name='installer_verb';Expression={
 if($_.CommandLine -match '--installer-[a-z-]+|__windows-(?:install|uninstall)'){$Matches[0]}
}},@{Name='created_at';Expression={$_.CreationDate.ToUniversalTime().ToString('o')}})
$tasks=@(Get-ScheduledTask | Where-Object {
 $principal=$_.Principal.UserId
 $ours=$principal -eq $identity.User.Value -or $principal -eq $identity.Name -or $principal -eq $env:USERNAME
 $_.TaskName -ne $ownTask -and $ours -and $_.TaskName -like 'HiRoute*' -and
 ($_.TaskName -notlike 'HiRoute-Dev-*' -or $_.State -eq 'Running')
} | Select-Object TaskName,State)
@{administrator_groups=($identity.Groups.Value -contains 'S-1-5-32-544');processes=$processes;tasks=$tasks}|ConvertTo-Json -Depth 5 -Compress
"""
    return json.loads(ps(script))


def admit_test_profile(request, home, localappdata):
    # These native ACL cases create only tempfile-owned artifacts. They can share
    # a logged-on account with an installed product without touching its profile.
    artifact_cases = {
        'windows_artifact_restart_restores_original_content_and_native_acl',
        'windows_acl_change_loses_compensation_ownership_even_with_equal_content',
        'windows_backup_authentication_binds_native_permission_facts',
        'windows_owned_directory_cleanup_preserves_user_additions_and_replacements',
    }
    prefix = 'control::windows_artifacts_tests::'
    if (request['binary'] == 'hiroute_local_storage.exe' and not request['ignored']
            and request['case'] in {prefix + name for name in artifact_cases}):
        return 'temporary_artifact_component'
    # Never manufacture HOME or remove an existing installation to make a gate pass.
    for path in (localappdata / 'HiRoute/cli', home / '.local/share/hiroute/standalone.json',
                 home / '.local/bin/hiroute.exe'):
        if path.exists():
            raise ValueError('Disposable installed-product gate found an existing installation')
    return 'disposable_installation_profile'


def worker(request):
    config = request['config']
    root = plain(config['root'])
    run = directory(root, request['run_id'])
    result = dict(passed=False, case=request['case'], driver_sha256=request['driver_sha256'],
                  driver_revision=request['driver_revision'],
                  os=platform.platform(), machine=platform.machine(), source_evidence='development only')
    started = time.monotonic()
    child = None
    try:
        if digest(Path(__file__)) != request['driver_sha256']:
            raise ValueError('Worker driver changed')
        session = ctypes.c_ulong()
        if not ctypes.windll.kernel32.ProcessIdToSessionId(os.getpid(), ctypes.byref(session)):
            raise OSError('Cannot resolve actual Windows session')
        user = os.environ['USERNAME']
        if user.casefold() != config['user'].split('\\')[-1].casefold() or session.value != config['session_id']:
            raise ValueError('The configured account is not in the expected interactive session')
        result.update(session_id=session.value, administrator=bool(ctypes.windll.shell32.IsUserAnAdmin()))
        harness = root / 'bundles' / request['harness']
        product = root / 'bundles' / request['product']
        hm, pm = manifest(harness), manifest(product)
        result.update(harness_revision=hm['source_revision'], product_revision=pm['source_revision'],
                      harness_files=hm['files'], product_files=pm['files'])
        if request['binary'] not in hm['files'] or not {'hiroute.exe', 'hirouted.exe'} <= set(pm['files']):
            raise ValueError('The selected manifests do not include required binaries')
        if request.get('operation') == 'desktop':
            if pm.get('installer_format') != 'nsis' or not re.fullmatch(r'[a-f0-9]{64}', pm.get('shell_sha256', '')):
                raise ValueError('Desktop acceptance requires a measured NSIS bundle')
            activity = profile_activity(config, request['run_id'])
            result['profile_activity'] = activity
            outcome = desktop_helper(request).execute(request, run, product, pm, profile_helper(request),
                                                     activity, manifest, root)
            assessment = {'codex-call':'native_codex_gateway_call', 'codex-inference':'native_codex_gateway_inference',
                          'codex-two-turn':'native_codex_gateway_two_turn',
                          'installer-quarantine':'preparation_only',
                          'network-probe':'diagnostic_only'}
            result.update(passed=True, assessment=assessment.get(request.get('desktop_action'),'native_desktop_observation'), desktop=outcome)
            return 0
        if request.get('operation') == 'inspect':
            result.update(passed=True, assessment='inspection_only',
                          installation=profile_helper(request).inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA'])),
                          profile_activity=profile_activity(config, request['run_id']))
            return 0
        if request.get('operation') in ('archive', 'uninstall', 'probe', 'uninstall-adapter'):
            parent = directory(root, request['inspection_run'])
            inspected = json.loads((parent / 'result.json').read_text(encoding='utf-8'))
            original = json.loads((parent / 'request.json').read_text(encoding='utf-8'))
            if (not inspected.get('passed') or inspected.get('assessment') != 'inspection_only'
                    or original.get('operation') != 'inspect'
                    or original['config']['user'] != config['user']
                    or original['config']['session_id'] != config['session_id']
                    or original['product'] != request['product']
                    or inspected.get('administrator') is not False
                    or time.time() - (parent / 'result.json').stat().st_mtime > 3600):
                raise ValueError('A recent inspection of this standard account and product is required')
            activity = profile_activity(config, request['run_id'])
            result['profile_activity'] = activity
            if request['operation'] in ('uninstall', 'probe', 'uninstall-adapter'):
                helper = profile_helper(request)
                uninstaller, installed = helper.candidate_uninstaller(
                    Path.home(), Path(os.environ['LOCALAPPDATA']), inspected['installation'],
                    pm, request['shell_sha256'], activity)
                before = helper.locations(Path.home(), Path(os.environ['LOCALAPPDATA']))
                if request['operation'] == 'probe':
                    probes = []
                    for verb in ('--installer-preflight', '--installer-options'):
                        completed = subprocess.run([str(before['programs'] / 'hiroute-shell.exe'), verb],
                                                   stdin=subprocess.DEVNULL, capture_output=True,
                                                   timeout=60, text=True, errors='replace')
                        probes.append(dict(verb=verb, exit_code=completed.returncode,
                                           stdout=completed.stdout[:4096], stderr=completed.stderr[:4096]))
                    result.update(passed=True, assessment='read_only_probe', probes=probes)
                    return 0
                if request['operation'] == 'uninstall-adapter':
                    source = before['programs'] / 'hiroute-shell.exe'
                    adapter = run / 'uninstall-adapter.exe'
                    shutil.copyfile(plain(source), adapter)
                    if digest(adapter) != request['shell_sha256']:
                        raise ValueError('Staged native adapter differs from the installed shell')
                    with (run / 'adapter.stdout').open('xb') as out, (run / 'adapter.stderr').open('xb') as err:
                        completed = subprocess.run([str(adapter), '--installer-uninstall'], stdin=subprocess.DEVNULL,
                                                   stdout=out, stderr=err, timeout=150)
                    result.update(passed=completed.returncode == 0, assessment='native_uninstall_adapter_only',
                                  native_entry='packaged --installer-uninstall', exit_code=completed.returncode,
                                  installation=helper.inspect_installation(Path.home(), Path(os.environ['LOCALAPPDATA'])))
                    return 0 if result['passed'] else 1
                staged = run / 'nsis-uninstaller.exe'
                shutil.copyfile(plain(uninstaller), staged)
                if digest(staged) != digest(uninstaller):
                    raise ValueError('Staged NSIS uninstaller differs from its verified inventory')
                deadline = time.monotonic() + request['timeout']
                with (run / 'uninstall.stdout').open('xb') as out, (run / 'uninstall.stderr').open('xb') as err:
                    completed = subprocess.run(nsis_uninstall_command(staged, before['programs']), executable=str(staged),
                                               stdin=subprocess.DEVNULL, stdout=out, stderr=err, timeout=150, check=True)
                removed = [Path(inspected['installation']['marker']['path']), installed,
                           before['stable_cli'], before['programs'] / 'hiroute-shell.exe',
                           Path(inspected['installation']['definition']['path'])]
                while any(path.exists() for path in removed):
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Candidate uninstall did not remove its owned installation')
                    time.sleep(0.2)
                result.update(passed=True, assessment='preparation_only', native_entry='NSIS /S',
                              exit_code=completed.returncode, versioned_package_removed=True,
                              state_preserved=before['state'].exists(),
                              inspection_sha256=digest(parent / 'result.json'))
                return 0
            archived = profile_helper(request).archive_orphaned_data(
                Path.home(), Path(os.environ['LOCALAPPDATA']), run,
                inspected['installation'], activity)
            result.update(passed=True, assessment='preparation_only', archival=archived,
                          inspection_sha256=digest(parent / 'result.json'))
            return 0
        home = Path.home()
        result['execution_scope'] = admit_test_profile(request, home, Path(os.environ['LOCALAPPDATA']))
        if (request['binary'] == 'windows_desktop.exe' and
                request['case'] == 'actual_nsis_silent_install_repeat_console_and_uninstall_preserve_data'):
            profile_helper(request).require_fresh_nsis_packages(home)
        environment = os.environ.copy()
        environment.update(config['environment'])
        for path in config['environment'].values():
            if not Path(path).exists():
                raise ValueError('Configured native client prerequisite is absent')
        fixture = Path(environment['HIROUTE_WINDOWS_SOURCE_ROOT']) / 'tools/windows-agent-fixtures'
        dependencies = json.loads((fixture / 'package.json').read_text(encoding='utf-8'))['dependencies']
        for name, version in dependencies.items():
            actual = json.loads((fixture / 'node_modules' / name / 'package.json').read_text(encoding='utf-8'))
            if actual['version'] != version:
                raise ValueError('Installed native client differs from pinned fixture')
        result['package_lock_sha256'] = digest(fixture / 'package-lock.json')
        result['clients'] = {name: digest(Path(path)) for name, path in config['environment'].items()
                             if Path(path).is_file()}
        environment.update(HIROUTE_VALIDATION_PRODUCT_BIN_DIR=str(product),
                           HIROUTE_VALIDATION_EVIDENCE_DIR=str(run),
                           HIROUTE_WORKER_CLI_BINARY=str(product / 'hiroute.exe'),
                           HIROUTE_WORKER_DAEMON_BINARY=str(product / 'hirouted.exe'))
        environment['PATH'] = ';'.join(config['path']) + ';' + environment['PATH']
        binary = harness / request['binary']
        listed = subprocess.run([str(binary), '--list', '--format', 'terse'], env=environment,
                                capture_output=True, text=True, timeout=30, check=True)
        if listed.stdout.splitlines().count(request['case'] + ': test') != 1:
            raise ValueError('Exact required test is missing or ambiguous')
        args = [str(binary), request['case'], '--exact', '--nocapture', '--test-threads=1']
        if request['ignored']:
            args.append('--ignored')
        with (run / 'stdout.log').open('xb') as output, (run / 'stderr.log').open('xb') as error:
            child = subprocess.Popen(args, env=environment, stdout=output, stderr=error)
            while child.poll() is None:
                if time.monotonic() - started > request['timeout']:
                    subprocess.run(['taskkill.exe', '/PID', str(child.pid), '/T', '/F'],
                                   capture_output=True, timeout=30, check=True)
                    child.wait(timeout=30)
                    raise TimeoutError('Whole test process exceeded its watchdog')
                diagnostics = Path(os.environ['LOCALAPPDATA']) / 'HiRoute/cli/diagnostics'
                for path in (diagnostics.rglob('*.jsonl') if
                             result['execution_scope'] == 'disposable_installation_profile' else ()):
                    try:
                        if path.stat().st_size <= 8 * 1024 * 1024:
                            name = hashlib.sha256(str(path.relative_to(diagnostics)).encode()).hexdigest()[:16]
                            shutil.copyfile(plain(path), run / ('diagnostics-' + name + '.jsonl'))
                    except (PermissionError, FileNotFoundError):
                        pass
                time.sleep(0.5)
        stdout = (run / 'stdout.log').read_text(encoding='utf-8', errors='replace')
        result.update(exit_code=child.returncode, passed=child.returncode == 0 and
                      bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', stdout)))
    except Exception as error:
        result.update(error_type=type(error).__name__, error=str(error))
    finally:
        if child is not None and child.poll() is None:
            subprocess.run(['taskkill.exe', '/PID', str(child.pid), '/T', '/F'], capture_output=True, timeout=30)
            child.wait(timeout=30)
        result['seconds'] = round(time.monotonic() - started, 3)
        temporary = run / 'result.tmp'
        temporary.write_text(json.dumps(result, indent=2), encoding='utf-8')
        temporary.replace(run / 'result.json')
    return 0 if result['passed'] else 1


def finish(root, run_id):
    run = directory(root, run_id)
    request = json.loads((run / 'request.json').read_text(encoding='utf-8'))
    config = request['config']
    expected = subprocess.list2cmdline([request['driver_path'], '--worker', str(run / 'request.json')])
    script = "$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"
    script += '$t=Get-ScheduledTask -TaskName ' + literal(task_name(run_id)) + ';'
    script += "if($t.State -ne 'Ready' -or $t.Actions.Execute -ne " + literal(worker_image(request))
    script += ' -or $t.Actions.Arguments -ne ' + literal(expected) + "){throw 'Owned task is active or differs'};"
    script += '$t|Unregister-ScheduledTask -Confirm:$false'
    ps(script)
    lock = plain(root / 'active.json')
    if json.loads(lock.read_text())['run_id'] != run_id:
        raise ValueError('Active owner differs; lock retained')
    lock.unlink()
    return dict(run_id=run_id, task_removed=True, evidence_retained=True,
                native_configuration='not modified by finish')


def stop_interrupted_daemon(pid, image, config, lower, upper):
    """Terminate only the retained image born during this stopped fixture run."""
    from ctypes import wintypes
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                                wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE, *([ctypes.POINTER(wintypes.FILETIME)] * 4)]
    kernel.ProcessIdToSessionId.argtypes = [wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    handle = kernel.OpenProcess(0x100000 | 0x1000 | 1, False, pid)
    if not handle:
        raise OSError(ctypes.get_last_error(), 'Cannot retain interrupted daemon')
    try:
        name, length, session = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768), wintypes.DWORD()
        born, exited, system, user = (wintypes.FILETIME() for _ in range(4))
        if (not kernel.QueryFullProcessImageNameW(handle, 0, name, ctypes.byref(length)) or
                not kernel.ProcessIdToSessionId(pid, ctypes.byref(session)) or
                not kernel.GetProcessTimes(handle, ctypes.byref(born), ctypes.byref(exited),
                                           ctypes.byref(system), ctypes.byref(user))):
            raise OSError('Cannot verify interrupted daemon identity')
        birth = ((born.dwHighDateTime << 32) | born.dwLowDateTime) / 10000000 - 11644473600
        if (Path(name.value) != image or session.value != config['session_id'] or
                not lower <= birth <= upper):
            raise ValueError('Daemon image, session or birth differs from the stopped run')
        if kernel.WaitForSingleObject(handle, 0) != 0:
            if not kernel.TerminateProcess(handle, 1) or kernel.WaitForSingleObject(handle, 5000) != 0:
                raise OSError('Interrupted daemon did not stop')
    finally:
        kernel.CloseHandle(handle)


def recover_interrupted_fixture(root, run, request, stop_only=False):
    """Preserve an interrupted disposable fixture after its whole-process watchdog."""
    home = Path.home()
    marker = plain(home / '.local/share/hiroute/standalone.json')
    state = plain(Path(os.environ['LOCALAPPDATA']) / 'HiRoute/cli')
    lower = (run / 'request.json').stat().st_mtime
    upper = time.time() if stop_only else (run / 'result.json').stat().st_mtime
    if marker.stat().st_nlink != 1 or not lower <= marker.stat().st_mtime <= upper:
        raise ValueError('Installation marker is outside the stopped run')
    record = json.loads(marker.read_text(encoding='utf-8'))
    install = plain(Path(record['install_root']))
    if (record.get('schema_version') != 'hiroute.standalone-install/v1' or
            record.get('target') != 'x86_64-pc-windows-msvc' or
            not re.fullmatch(r'Windows headless 测试 [1-9][0-9]*', install.name) or
            install.parent != home / '.local/share/hiroute' or
            Path(record['service_definition']) != install / 'fixture-task.xml' or
            record.get('installed_skills', []) or record.get('cpa_binary') or record.get('cpa_sha256')):
        raise ValueError('Installation is not the disposable gateway fixture')
    product = manifest(root / 'bundles' / request['product'])
    entries = []
    for name in ('hiroute.exe', 'hirouted.exe'):
        image, stable = plain(install / name), plain(home / '.local/bin' / name)
        if (digest(image) != product['files'][name]['sha256'] or
                not image.samefile(stable) or image.stat().st_nlink != 2):
            raise ValueError('Installed images differ from the owned product bundle')
        entries.append(stable)
    harness = plain(root / 'bundles' / request['harness'] / request['binary'])
    paths = ','.join(literal(p) for p in [harness, install / 'hiroute.exe', install / 'hirouted.exe', *entries])
    daemon = install / 'hirouted.exe'
    value = ps("$ErrorActionPreference='Stop';$path=" + literal(daemon) +
               ';@(Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -eq $path} | '
               'ForEach-Object {[int]$_.ProcessId}) | ConvertTo-Json -Compress')
    pids = json.loads(value) if value else []
    for pid in pids if isinstance(pids, list) else [pids]:
        stop_interrupted_daemon(pid, daemon, request['config'], lower, upper)
    if stop_only:
        return dict(run_id=request['run_id'], runtime_stopped=True, test_owner_retained=True)
    live = ps("$ErrorActionPreference='Stop';$paths=@(" + paths +
              ');@(Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -in $paths}).Count')
    if live != '0':
        raise ValueError('Owned fixture still has live processes; preserve it')
    moves = [(marker, run / 'interrupted-standalone.json'),
             (install, run / 'interrupted-installation'), (state, run / 'interrupted-state')]
    if any(target.exists() for _, target in moves):
        raise ValueError('Interrupted fixture evidence already exists')
    # Admit all file moves first, preserve bytes in private evidence, and remove
    # only verified links to the owned images.
    for entry in entries:
        entry.unlink()
    for source, target in moves:
        source.rename(target)
    return dict(run_id=request['run_id'], preserved=[p.name for _, p in moves])


def stop_panicking_runtime(root, run_id):
    run = directory(root, run_id)
    request = json.loads((run / 'request.json').read_text(encoding='utf-8'))
    if ((run / 'result.json').exists() or
            json.loads((root / 'active.json').read_text())['run_id'] != run_id or
            task_state(run_id)['state'] != 'Running'):
        raise ValueError('Runtime stop requires the active nonterminal run')
    if (request['binary'] != 'windows_headless.exe' or request['case'] not in (
            'actual_windows_complete_public_headless_business',
            'actual_windows_public_headless_business_without_claude',
            'windows_gateway_foreground_recovery::actual_windows_foreground_gateway_recovery') or
            os.environ['USERNAME'].casefold() != request['config']['user'].casefold()):
        raise ValueError('Runtime stop requires the recognized disposable case and account')
    error = (run / 'stderr.log').read_text(encoding='utf-8', errors='replace')
    if not re.search(r"^thread '" + re.escape(request['case']) + r"' .* panicked at ", error, re.M):
        raise ValueError('The selected test has not reported a panic; preserve its runtime')
    return recover_interrupted_fixture(root, run, request, stop_only=True)


def recover(root, run_id):
    """Preserve only identifiable files left by this stopped disposable business case."""
    run = directory(root, run_id)
    request = json.loads((run / 'request.json').read_text(encoding='utf-8'))
    result = json.loads((run / 'result.json').read_text(encoding='utf-8'))
    if (root / 'active.json').exists() or task_state(run_id)['state'] != 'absent':
        raise ValueError('Finish the stopped run before recovering its owned configuration')
    if result['passed'] or request['case'] not in (
            'actual_windows_complete_public_headless_business',
            'actual_windows_public_headless_business_without_claude',
            'windows_gateway_foreground_recovery::actual_windows_foreground_gateway_recovery'):
        raise ValueError('Recovery is limited to failed disposable business cases')
    if os.environ['USERNAME'].casefold() != request['config']['user'].casefold():
        raise ValueError('Recovery must use the same native account')
    if request['case'] == 'windows_gateway_foreground_recovery::actual_windows_foreground_gateway_recovery':
        if request['binary'] != 'windows_headless.exe' or result.get('error_type') != 'TimeoutError':
            raise ValueError('Installed fixture recovery requires the interrupted gateway case')
        return recover_interrupted_fixture(root, run, request)
    home = Path.home()
    state = Path(os.environ['LOCALAPPDATA']) / 'HiRoute/cli'
    if any(p.exists() for p in (state, home / '.local/bin/hiroute.exe',
                               home / '.local/share/hiroute/standalone.json')):
        raise ValueError('An installed product remains; use its public restore operation')
    lower, upper = (run / 'request.json').stat().st_mtime, (run / 'result.json').stat().st_mtime
    candidates = [('codex-config.toml', home / '.codex/config.toml'),
                  ('collaboration-skill.md', home / '.agents/skills/hiroute-collaboration/SKILL.md'),
                  ('claude-settings.json', home / '.claude/settings.json')]
    admitted = []
    for name, path in candidates:
        if not path.exists():
            continue
        plain(path)
        info = path.stat()
        if info.st_nlink != 1 or not lower <= info.st_mtime <= upper or info.st_size > 1024 * 1024:
            raise ValueError('Native configuration identity or modification time is outside this run')
        body = path.read_bytes()
        if name == 'codex-config.toml':
            import tomllib
            value = tomllib.loads(body.decode('utf-8'))
            if set(value) != {'model', 'model_provider', 'model_catalog_json', 'model_providers'} or value['model_provider'] != 'hiroute':
                raise ValueError('Unrecognized Codex configuration; retain it')
            Path(value['model_catalog_json']).relative_to(state / 'storage/managed-artifacts/agent-connections')
        elif name == 'claude-settings.json':
            if json.loads(body) != {'windows_headless_preserved': True}:
                raise ValueError('Claude configuration is not the untouched disposable marker')
        elif (list(path.parent.iterdir()) != [path] or
              b'hiroute worker plans' not in body or b'hiroute worker exec' not in body):
            raise ValueError('Unrecognized collaboration skill; retain it')
        destination = run / ('recovered-' + name)
        if destination.exists():
            raise ValueError('Recovery evidence already exists')
        admitted.append((path, destination, digest(path)))
    for path, destination, checksum in admitted:
        if digest(path) != checksum:
            raise ValueError('Native file changed during recovery')
        path.rename(destination)
        if path.name == 'SKILL.md':
            path.parent.rmdir()
    return dict(run_id=run_id, preserved=[p.name for _, p, _ in admitted])


def execute(request):
    root = plain(request['config']['root'])
    action = request['action']
    if action == 'doctor':
        return dict(os=platform.platform(), machine=platform.machine(), python=sys.version.split()[0],
                    configured_session=request['config']['session_id'],
                    sessions=subprocess.run(['query.exe', 'user'], capture_output=True, text=True,
                                            errors='replace', timeout=15).stdout,
                    active=json.loads((root / 'active.json').read_text()) if (root / 'active.json').exists() else None)
    if action == 'stage':
        return stage(root, request['archive'], request['sha256'])
    if action == 'run':
        return start(root, request)
    run = directory(root, request['run_id'])
    if action in ('status', 'wait'):
        if action == 'wait':
            end = time.monotonic() + min(50, max(1, request['timeout']))
            while not (run / 'result.json').exists() and time.monotonic() < end:
                time.sleep(1)
        return status(root, request['run_id'])
    if action == 'logs':
        return {name: (run / name).read_text(encoding='utf-8', errors='replace').splitlines()[-request['tail']:]
                for name in ('stdout.log', 'stderr.log') if (run / name).exists()}
    if action == 'collect':
        if task_state(request['run_id'])['state'] == 'Running':
            raise ValueError('Wait for the runner to stop before collecting final evidence')
        return dict(directory=str(run), files=[p.name for p in run.iterdir() if p.is_file()])
    if action == 'recover':
        return recover(root, request['run_id'])
    if action == 'stop-runtime':
        return stop_panicking_runtime(root, request['run_id'])
    if action == 'finish':
        return finish(root, request['run_id'])
    raise ValueError('Unknown Windows development action')


if __name__ == '__main__':
    if sys.platform != 'win32':
        raise SystemExit('Native Windows required')
    if sys.argv[1] == '--worker':
        sys.exit(worker(json.loads(Path(sys.argv[2]).read_text(encoding='utf-8'))))
    path = plain(sys.argv[1])
    if digest(path) != sys.argv[2]:
        raise SystemExit('Request hash differs')
    print(json.dumps(execute(json.loads(path.read_text(encoding='utf-8')))))
