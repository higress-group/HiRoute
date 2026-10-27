#!/usr/bin/env python3
"""Windows development-machine transport; personal locations stay in HOME."""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PureWindowsPath
import re
import subprocess
import sys
import tempfile
import uuid
import zipfile

REPO = Path(__file__).resolve().parent.parent
CONFIG = Path.home() / '.config/hiroute/windows-validation.json'
ENVIRONMENT = {
    'HIROUTE_WINDOWS_SOURCE_ROOT', 'HIROUTE_WORKER_NODE', 'HIROUTE_NATIVE_CODEX',
    'HIROUTE_WORKER_CODEX_BINARY', 'HIROUTE_WORKER_CLAUDE_BINARY',
    'HIROUTE_WORKER_QODER_BINARY', 'HIROUTE_WORKER_CODEX_ACP_ADAPTER',
    'HIROUTE_WORKER_CLAUDE_ACP_ADAPTER', 'HIROUTE_WORKER_PI_BINARY',
    'HIROUTE_WORKER_DSH_BINARY', 'HIROUTE_WORKER_OPENCODE_BINARY',
}


def digest(path):
    with Path(path).open('rb') as stream:
        value = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
        return value.hexdigest()


def windows_path(value):
    if not isinstance(value, str) or not PureWindowsPath(value).is_absolute() or any(
            c in value for c in '\x00\r\n') or '..' in PureWindowsPath(value).parts:
        raise ValueError('Expected an absolute Windows path without parent traversal')
    return value


def remote_evidence_path(directory, name):
    # OpenSSH's SFTP path grammar requires forward slashes, including when
    # pathlib on the Windows executor returned a native backslash path.
    return PureWindowsPath(windows_path(directory)).as_posix() + '/' + name


def configuration(path):
    path = path.expanduser().resolve(strict=True)
    path.relative_to(Path.home().resolve())
    if path.is_relative_to(REPO):
        raise ValueError('Machine configuration belongs in HOME, outside the checkout')
    if os.name != 'nt' and path.stat().st_mode & 0o077:
        raise ValueError('Machine configuration requires mode 0600')
    value = json.loads(path.read_text())
    required = {'version', 'host', 'identity_file', 'python', 'root', 'user',
                'session_id', 'environment', 'path'}
    if set(value) != required or value['version'] != 1:
        raise ValueError('Expected windows-validation.json version 1 fields')
    if not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.@:-]*', value['host']):
        raise ValueError('Expected an SSH alias or user@host')
    identity = Path(value['identity_file']).expanduser().resolve(strict=True)
    identity.relative_to(Path.home().resolve())
    value['identity_file'] = str(identity)
    for key in ('python', 'root'):
        windows_path(value[key])
    if not re.fullmatch(r'[A-Za-z0-9_.\\@-]+', value['user']):
        raise ValueError('Expected an explicit Windows account')
    if type(value['session_id']) is not int or value['session_id'] < 1:
        raise ValueError('An existing interactive session ID is required')
    if not isinstance(value['environment'], dict) or set(value['environment']) - ENVIRONMENT:
        raise ValueError('Only declared client paths are accepted; credentials are not configuration')
    if not isinstance(value['path'], list):
        raise ValueError('path must contain Windows discovery directories')
    for entry in [*value['environment'].values(), *value['path']]:
        windows_path(entry)
    return value


def powershell(script):
    return 'powershell.exe -NoProfile -NonInteractive -EncodedCommand ' + base64.b64encode(
        script.encode('utf-16le')).decode()


def ps_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def python_command(binary, code):
    # Windows PowerShell native argv processing can remove Python's embedded
    # double quotes. Transfer the small bootstrap as data, not source in argv.
    encoded = base64.b64encode(code.encode('utf-8')).decode()
    return ('& ' + ps_literal(binary) + ' -c "import base64;exec(base64.b64decode(\'' +
            encoded + '\'))"; exit $LASTEXITCODE')


def bootstrap_code(root, entries, publish=None):
    """Publish a content-addressed helper once; wait/status never rewrite it."""
    return ('bootstrap_root=' + repr(root) + '\nbootstrap_entries=' + repr(entries) +
            '\nbootstrap_publication=' + repr(publish) + '\n') + '''from pathlib import Path
import hashlib,json,stat
def plain(value):
 p=Path(value)
 for entry in [p,*p.parents]:
  try: info=entry.lstat()
  except FileNotFoundError: continue
  if stat.S_ISLNK(info.st_mode) or getattr(info,'st_file_attributes',0)&1024: raise ValueError('Reparse bootstrap path')
 return p
root=plain(bootstrap_root)
root.mkdir(parents=True,exist_ok=True)
def verify(value,checksum):
 p=plain(value)
 if not p.is_file() or p.stat().st_nlink!=1 or hashlib.sha256(p.read_bytes()).hexdigest()!=checksum: raise ValueError('Bootstrap helper identity changed')
if bootstrap_publication:
 source=plain(bootstrap_publication['temporary']);target=plain(bootstrap_publication['path']);checksum=bootstrap_publication['sha256']
 verify(source,checksum)
 if target.exists():
  verify(target,checksum)
  source.unlink()
 else: source.rename(target)
present=[]
for entry in bootstrap_entries:
 p=plain(entry['path'])
 if p.exists():
  verify(p,entry['sha256']);present.append(entry['sha256'])
print(json.dumps(present))
'''


class Remote:
    def __init__(self, config):
        self.config = config
        self.options = ['-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes',
                        '-o', 'IdentitiesOnly=yes', '-o', 'ConnectTimeout=15',
                        '-i', config['identity_file']]

    def execute(self, script, timeout=65):
        command = powershell(script)
        payload = None
        if len(command) > 6000:
            # Windows OpenSSH may enter through cmd.exe's 8191-character limit.
            # A length-checked line avoids waiting for SSH/PowerShell to forward
            # EOF before the remote command can run. Never replay a timeout.
            encoded = base64.b64encode(script.encode('utf-8')).decode()
            payload = encoded + '\n'
            command = powershell(
                "$ErrorActionPreference='Stop';"
                "$line=[Console]::In.ReadLine();"
                "if($null -eq $line -or $line.Length -ne " + str(len(encoded)) + "){throw 'Incomplete source frame'};"
                "$source=[Text.Encoding]::UTF8.GetString("
                "[Convert]::FromBase64String($line));"
                "& ([ScriptBlock]::Create($source))")
        result = subprocess.run(['ssh', *self.options, self.config['host'], command],
                                input=payload, capture_output=True, text=True,
                                errors='replace', timeout=timeout)
        if result.returncode:
            raise RuntimeError('Windows transport failed; no automatic replay was attempted: ' +
                               result.stderr[-2000:])
        return result.stdout

    def upload(self, source, destination):
        subprocess.run(['scp', '-q', *self.options, str(source),
                        self.config['host'] + ':' + destination], check=True, timeout=180)

    def bootstrap(self):
        # Uploading source avoids Windows command-line length limits. Only a small
        # bootstrap enters argv; request data is a separately hashed JSON file.
        root = self.config['root']
        source = REPO / 'scripts/windows-dev-host.py'
        self.driver_hash = digest(source)
        self.driver = root + '/driver-' + self.driver_hash + '.py'
        profile = REPO / 'scripts/windows-dev-profile.py'
        self.profile_hash = digest(profile)
        self.profile_helper = root + '/profile-' + self.profile_hash + '.py'
        desktop = REPO / 'scripts/windows-desktop-session.py'
        self.desktop_hash = digest(desktop)
        self.desktop_helper = root + '/desktop-' + self.desktop_hash + '.py'
        sources = [(source,self.driver,self.driver_hash),(profile,self.profile_helper,self.profile_hash),
                   (desktop,self.desktop_helper,self.desktop_hash)]
        entries = [dict(path=path,sha256=checksum) for _,path,checksum in sources]
        present = json.loads(self.execute(python_command(self.config['python'], bootstrap_code(root, entries))))
        for local,path,checksum in sources:
            if checksum in present:
                continue
            temporary = path + '.upload-' + uuid.uuid4().hex
            self.upload(local, temporary)
            publication = dict(path=path,temporary=temporary,sha256=checksum)
            self.execute(python_command(self.config['python'], bootstrap_code(root, entries, publication)))

    def call(self, action, **arguments):
        paths = ['scripts/windows-dev.py', 'scripts/windows-dev-host.py',
                 'scripts/windows-dev-profile.py', 'scripts/windows-desktop-session.py', 'scripts/validation.py']
        if action == 'run':
            subprocess.run(['git', 'ls-files', '--error-unmatch', *paths], cwd=REPO,
                           stdout=subprocess.DEVNULL, check=True)
            if subprocess.check_output(['git', 'status', '--porcelain', '--', *paths], cwd=REPO):
                raise ValueError('Commit the runner before native product execution')
        request = dict(action=action, config=self.config, driver_sha256=self.driver_hash,
                       profile_helper_path=self.profile_helper, profile_helper_sha256=self.profile_hash,
                       desktop_helper_path=self.desktop_helper, desktop_helper_sha256=self.desktop_hash,
                       driver_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
                       driver_path=self.driver, **arguments)
        # SSH identity and host never need to reach the Windows execution process.
        request['config'] = {k: v for k, v in self.config.items()
                             if k not in ('host', 'identity_file')}
        with tempfile.TemporaryDirectory(prefix='hiroute-windows-request-') as temporary:
            path = Path(temporary) / 'request.json'
            path.write_text(json.dumps(request), encoding='utf-8')
            checksum = digest(path)
            remote = self.config['root'] + '/request-' + checksum + '.json'
            self.upload(path, remote)
        code = ('from pathlib import Path;import hashlib,runpy,sys; p=Path(' + repr(self.driver) +
                ');assert hashlib.sha256(p.read_bytes()).hexdigest()==' + repr(self.driver_hash) +
                ';sys.argv=[str(p),' + repr(remote) + ',' + repr(checksum) + '];runpy.run_path(str(p),run_name="__main__")')
        output = self.execute(python_command(self.config['python'], code))
        return json.loads(output)


def bundle(result_path, log_path, output):
    result = json.loads(result_path.read_text())
    sha = result['sha']
    if result['process_exit'] != 0 or not re.fullmatch('[a-f0-9]{40}', sha):
        raise ValueError('A successful revision-bound managed build is required')
    checkout = Path(result['checkout']).resolve(strict=True)
    if subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip() != sha:
        raise ValueError('Build checkout has moved; export before continuing it')
    images = {}
    for line in log_path.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('reason') != 'compiler-artifact' or not row.get('executable'):
            continue
        path = Path(row['executable']).resolve(strict=True)
        path.relative_to(checkout / 'target/x86_64-pc-windows-msvc')
        name = row['target']['name'] + '.exe'
        if name in images and images[name] != path:
            raise ValueError('Ambiguous binary target; export one explicit build')
        images[name] = path
    if not images:
        raise ValueError('Build log selected zero executable artifacts')
    manifest = dict(source_revision=sha, target='x86_64-pc-windows-msvc',
                    producer_platform=sys.platform, native_ci_assessment='not_assessed',
                    run_id=result.get('id'), command=result.get('command'),
                    cargo_lock_sha256=digest(checkout / 'Cargo.lock'), files={})
    with zipfile.ZipFile(output, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for name, path in images.items():
            if not re.fullmatch(r'[A-Za-z0-9_-]+\.exe', name):
                raise ValueError('Invalid executable name')
            manifest['files'][name] = dict(sha256=digest(path), bytes=path.stat().st_size)
            archive.write(path, name)
        archive.writestr('manifest.json', json.dumps(manifest))
    return dict(bundle_sha256=digest(output), source_revision=sha, files=list(images))


def bundle_desktop(result_path, log_path, output):
    """Export a native release candidate and its exact compiled installer test."""
    result = json.loads(result_path.read_text())
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    identity = result.get('desktop_result', {})
    shell_sha256 = identity.get('shell_image', {}).get('sha256', '')
    artifact = result.get('website_artifact', {})
    if (result.get('revision') != sha or result.get('target') != 'x86_64-pc-windows-msvc'
            or result.get('status') != 'awaiting_runtime' or result.get('package_integrity') != 'green'
            or identity.get('revision') != sha or identity.get('native_runtime') != 'not_executed'
            or not re.fullmatch('[a-f0-9]{64}', shell_sha256)
            or artifact.get('kind') != 'desktop' or artifact.get('format') != 'nsis'
            or artifact.get('platform') != 'Windows' or artifact.get('architecture') != 'x86_64'):
        raise ValueError('A matching unassessed native x64 NSIS candidate is required')
    installer = Path(result['assets']) / artifact['filename']
    installer.resolve(strict=True).relative_to(REPO.resolve() / 'target')
    if (digest(installer) != identity.get('installer_sha256') or digest(installer) != artifact.get('sha256')
            or installer.stat().st_size != identity.get('installer_size')):
        raise ValueError('Installer differs from the measured build identity')
    tests = set()
    for line in log_path.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if (row.get('reason') == 'compiler-artifact' and row.get('target', {}).get('name') == 'windows_desktop'
                and row.get('profile', {}).get('test') is True and row.get('executable')):
            tests.add(Path(row['executable']).resolve(strict=True))
    if len(tests) != 1:
        raise ValueError('Exactly one compiled Windows Desktop test artifact is required')
    binaries = REPO / 'target/x86_64-pc-windows-msvc/release'
    harness = tests.pop()
    harness.relative_to(binaries.resolve(strict=True))
    spec = importlib.util.spec_from_file_location('runtime_audit', Path(__file__).with_name('windows-native-runtime-audit.py'))
    audit = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(audit)
    imports = audit.pe_imports(harness.read_bytes())
    if imports['debug_crt'] or imports['release_vc_runtime']:
        raise ValueError('The native acceptance harness must use static CRT without external VC runtime DLLs')
    payload = REPO / 'apps/shell/src-tauri/resources/standalone'
    if digest(payload / 'package-manifest.json') != identity.get('standalone_manifest_sha256'):
        raise ValueError('Embedded package differs from the installer build identity')
    package = json.loads((payload / 'package-manifest.json').read_text())
    if package.get('revision') != sha or package.get('target') != result['target']:
        raise ValueError('Embedded package source differs from the installer')
    for name in ('hiroute.exe', 'hirouted.exe'):
        expected = package['files']['bin/' + name]
        if digest(payload / name) != expected['sha256'] or (payload / name).stat().st_size != expected['size']:
            raise ValueError('Embedded product bytes differ from the measured package')
    files = {'installer.exe': installer, 'windows_desktop.exe': harness,
             **{name: payload / name for name in ('hiroute.exe', 'hirouted.exe')}}
    manifest = dict(source_revision=sha, target='x86_64-pc-windows-msvc', installer_format='nsis',
                    shell_sha256=shell_sha256, native_ci_assessment='not_assessed', files={})
    with zipfile.ZipFile(output, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for name, path in files.items():
            manifest['files'][name] = dict(sha256=digest(path), bytes=path.stat().st_size)
            archive.write(path, name)
        archive.writestr('manifest.json', json.dumps(manifest))
    return dict(bundle_sha256=digest(output), source_revision=sha, files=list(files),
                native_acceptance='not_executed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=CONFIG)
    commands = parser.add_subparsers(dest='action', required=True)
    commands.add_parser('doctor')
    stage = commands.add_parser('stage')
    stage.add_argument('archive', type=Path)
    inspect = commands.add_parser('inspect-installation')
    inspect.add_argument('--product', required=True)
    archive = commands.add_parser('archive-installation-data')
    archive.add_argument('inspection_run')
    archive.add_argument('--product', required=True)
    uninstall = commands.add_parser('uninstall-candidate')
    uninstall.add_argument('inspection_run')
    uninstall.add_argument('--product', required=True)
    uninstall.add_argument('--shell-sha256', required=True)
    uninstall.add_argument('--probe-only', action='store_true')
    uninstall.add_argument('--adapter-diagnostic', action='store_true', help='Run the verified mutating shell uninstall adapter and retain its typed result; this is not a full NSIS uninstall')
    desktop = commands.add_parser('desktop-session', help='Measured installed Desktop and native UI acceptance')
    desktop.add_argument('--product', required=True)
    desktop.add_argument('--action', dest='desktop_action', required=True,
                         choices=('install', 'installer-resume', 'installer-quarantine', 'launch', 'snapshot', 'invoke', 'text', 'click', 'scroll', 'close', 'quit', 'tray-observe', 'desktop-state', 'cli', 'native-files', 'codex-call', 'codex-inference', 'codex-two-turn', 'network-probe', 'recovery-lock-ui', 'recovery-file-use-ui'))
    desktop.add_argument('--from-product', help='Exact installed bundle admitted before a cross-build upgrade')
    desktop.add_argument('--recover-product', help='Exact interrupted bundle, with a recent inspection, before normal NSIS recovery')
    desktop.add_argument('--observation-run')
    desktop.add_argument('--element', help='Runtime ID from a recent native UI observation')
    desktop.add_argument('--text')
    desktop.add_argument('--x', type=int)
    desktop.add_argument('--y', type=int)
    desktop.add_argument('--scroll-direction', choices=('up', 'down'))
    desktop.add_argument('--codex-profile', choices=('hiroute',), help='Use the product-owned Codex profile instead of the default configuration')
    desktop.add_argument('--source-id', help='One ready saved source for unauthenticated TLS diagnosis')
    desktop.add_argument('--native-tls', action='store_true', help='Compare the Windows system TLS verifier without changing trust policy')
    desktop.add_argument('--generated-uninstaller-sha256', help='Digest independently extracted from the verified failed NSIS artifact, for manual quarantine only')
    desktop.add_argument('--cli-args', type=Path, help='Private JSON array of public CLI arguments')
    run = commands.add_parser('run')
    run.add_argument('--harness', required=True)
    run.add_argument('--product', required=True)
    run.add_argument('--binary', default='windows_headless.exe')
    run.add_argument('--case', required=True)
    run.add_argument('--ignored', action='store_true')
    run.add_argument('--timeout', type=int, default=900, help='Whole test process watchdog, not CLI timeout')
    for action in ('status', 'wait', 'logs', 'finish', 'recover', 'stop-runtime'):
        command = commands.add_parser(action)
        command.add_argument('run_id')
        if action == 'wait':
            command.add_argument('--timeout', type=int, default=50, choices=range(1, 51))
        if action == 'logs':
            command.add_argument('--tail', type=int, default=60, choices=range(1, 201))
    collect = commands.add_parser('collect')
    collect.add_argument('run_id')
    collect.add_argument('--output', type=Path, required=True)
    export = commands.add_parser('bundle')
    export.add_argument('--result', type=Path, required=True)
    export.add_argument('--log', type=Path, required=True)
    export.add_argument('--output', type=Path, required=True)
    export = commands.add_parser('bundle-desktop')
    export.add_argument('--result', type=Path, required=True)
    export.add_argument('--log', type=Path, required=True)
    export.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.action == 'bundle-desktop':
        value = bundle_desktop(args.result, args.log, args.output)
    elif args.action == 'bundle':
        value = bundle(args.result, args.log, args.output)
    else:
        remote = Remote(configuration(args.config))
        remote.bootstrap()
        values = vars(args).copy()
        for key in ('config', 'action'):
            values.pop(key)
        if args.action == 'stage':
            checksum = digest(args.archive)
            destination = remote.config['root'] + '/upload-' + checksum + '.zip'
            remote.upload(args.archive, destination)
            values = dict(archive=destination, sha256=checksum)
        elif args.action == 'inspect-installation':
            values = dict(run_id=uuid.uuid4().hex, harness=args.product, product=args.product,
                          binary='hiroute.exe', case='installation_profile_inspection',
                          operation='inspect', ignored=False, timeout=120)
            args.action = 'run'
        elif args.action == 'desktop-session':
            if args.from_product:
                identifier = r'[a-f0-9]{12}-[a-f0-9]{12}'
                if not re.fullmatch(identifier, args.from_product):
                    raise ValueError('Expected an explicit prior product bundle')
            if args.recover_product and (args.desktop_action != 'install' or not args.from_product
                    or not re.fullmatch(r'[a-f0-9]{12}-[a-f0-9]{12}', args.recover_product)
                    or not re.fullmatch(r'[a-f0-9]{32}', args.observation_run or '')):
                raise ValueError('Prepared NSIS recovery requires exact products and a recent inspection')
            if args.desktop_action in ('invoke', 'text'):
                if (not args.observation_run or not re.fullmatch(r'[a-f0-9]{32}', args.observation_run)
                        or not args.element or not re.fullmatch(r'[0-9.]+', args.element)):
                    raise ValueError('Native interaction requires an observed Runtime ID and run')
            if args.desktop_action == 'network-probe' and (
                    not args.source_id or not re.fullmatch(r'source/[A-Za-z0-9._:-]{1,240}', args.source_id)):
                raise ValueError('TLS diagnostics require an explicit saved source identity')
            if args.desktop_action in ('installer-resume', 'installer-quarantine') and (
                    not args.from_product or not args.observation_run
                    or not re.fullmatch(r'[a-f0-9]{32}', args.observation_run)):
                raise ValueError('Installer resume requires the prior product and a recent installation inspection')
            if args.desktop_action == 'installer-quarantine' and not re.fullmatch(
                    r'[a-f0-9]{64}', args.generated_uninstaller_sha256 or ''):
                raise ValueError('Manual quarantine requires the independently measured generated-uninstaller digest')
            if args.desktop_action == 'text' and (args.text is None or len(args.text) > 4096):
                raise ValueError('Native text input requires bounded text')
            if args.desktop_action in ('click', 'scroll'):
                if (not args.observation_run or not re.fullmatch(r'[a-f0-9]{32}', args.observation_run)
                        or args.x is None or args.y is None or not 0 <= args.x < 4096 or not 0 <= args.y < 4096):
                    raise ValueError('Pointer input requires a recent observed window and bounded coordinates')
                if args.desktop_action == 'scroll' and args.scroll_direction is None:
                    raise ValueError('Scrolling requires an explicit direction')
            values.update(run_id=uuid.uuid4().hex, harness=args.product, binary='hiroute.exe',
                          case='native_desktop_' + args.desktop_action.replace('-', '_'), operation='desktop', ignored=False,
                          timeout={'install':810,'recovery-lock-ui':600,'recovery-file-use-ui':600,
                                   'codex-two-turn':420}.get(args.desktop_action,240))
            values.pop('cli_args')
            if args.desktop_action == 'cli':
                if args.cli_args is None:
                    raise ValueError('Public CLI acceptance requires a private argument file')
                values['cli_args'] = json.loads(args.cli_args.read_text(encoding='utf-8'))
            args.action = 'run'
        elif args.action in ('archive-installation-data', 'uninstall-candidate'):
            uninstall = args.action == 'uninstall-candidate'
            values = dict(run_id=uuid.uuid4().hex, harness=args.product, product=args.product,
                          binary='hiroute.exe', case='candidate_uninstallation' if uninstall else 'orphaned_installation_data_archival',
                          operation='uninstall' if uninstall else 'archive', inspection_run=args.inspection_run,
                          ignored=False, timeout=600 if uninstall else 300)
            if uninstall:
                if not re.fullmatch(r'[a-f0-9]{64}', args.shell_sha256):
                    raise ValueError('Expected the measured shell SHA-256')
                values['shell_sha256'] = args.shell_sha256
                if args.adapter_diagnostic:
                    if args.probe_only:
                        raise ValueError('Read-only probing and mutating adapter diagnostics are distinct operations')
                    values.update(operation='uninstall-adapter', case='native_uninstall_adapter_diagnostic')
                if args.probe_only:
                    values.update(operation='probe', case='candidate_read_only_probe')
            args.action = 'run'
        elif args.action == 'run':
            values['run_id'] = uuid.uuid4().hex
        elif args.action == 'collect':
            values.pop('output')
        value = remote.call(args.action, **values)
        if args.action == 'collect':
            args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
            for name in value['files']:
                if not re.fullmatch(r'[A-Za-z0-9_.-]+', name):
                    raise ValueError('Invalid evidence filename')
                destination = args.output / name
                subprocess.run(['scp', '-q', *remote.options, remote.config['host'] + ':' +
                                remote_evidence_path(value['directory'], name), str(destination)], check=True, timeout=180)
                destination.chmod(0o600)
    print(json.dumps(value, indent=2))
    if args.action in ('status', 'wait') and value.get('state') == 'failed':
        return 1
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
