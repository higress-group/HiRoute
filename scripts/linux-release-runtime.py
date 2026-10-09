#!/usr/bin/env python3
"""Install an exact archive in a private HOME and exercise production Local Control."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import signal
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(manifest, archive, abi, output):
    result = {'status': 'red', 'libc': platform.libc_ver(), 'architecture': platform.machine()}
    output.mkdir(parents=True, exist_ok=True)
    process = None
    private = None
    home = None
    diagnostics = None
    try:
        if tuple(platform.libc_ver()) != ('glibc', '2.31'):
            raise ValueError('runtime acceptance requires glibc 2.31')
        spec = importlib.util.spec_from_file_location('package_standalone', ROOT / 'package-standalone.py')
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        package = packager.verify(manifest, archive)
        if package['target'] != packager.host_target():
            raise ValueError('runtime architecture differs from package')
        checked = json.loads(abi.read_text())
        policy = json.loads((ROOT / 'linux-release-baseline.json').read_text())
        if (checked.get('status') != 'green' or checked.get('archive_sha256') != package['archive']['sha256']
                or checked.get('manifest_sha256') != digest(manifest)
                or checked.get('target') != package['target'] or checked.get('revision') != package['revision']
                or checked.get('baseline') != policy
                or any(checked.get('files', {}).get(name, {}).get('sha256') != package['files'][name]['sha256']
                       for name in ('bin/hiroute', 'bin/hirouted', 'libexec/cliproxyapi'))):
            raise ValueError('ABI evidence differs from archive')
        result.update(revision=package['revision'], target=package['target'],
                      archive_sha256=digest(archive), manifest_sha256=digest(manifest))
        private = tempfile.TemporaryDirectory(prefix='hiroute-minimum-runtime-')
        home = Path(private.name)
        environment = {'HOME': str(home), 'PATH': '/usr/local/bin:/usr/bin:/bin',
                       'LANG': 'C.UTF-8', 'LD_BIND_NOW': '1'}

        def run(*args, required=True):
            completed = subprocess.run(list(map(str, args)), env=environment, text=True,
                                       capture_output=True, timeout=30)
            with (output / 'commands.log').open('a') as log:
                log.write(f'$ {" ".join(map(str, args))}\n{completed.stdout}{completed.stderr}\n')
            if required:
                completed.check_returncode()
            return completed

        run(sys.executable, ROOT / 'install-standalone.py', 'install',
            '--manifest', manifest, '--archive', archive)
        cli = home / '.local/bin/hiroute'
        daemon = home / '.local/bin/hirouted'
        marker = json.loads((home / '.local/share/hiroute/standalone.json').read_text())
        if marker['version'] != package['version'] or marker['target'] != package['target']:
            raise ValueError('installed version/target differs from verified manifest')
        result.update(version=package['version'],
                      version_source='verified_manifest_and_installation_marker')
        cpa = Path(marker['cpa_binary'])
        installed = {'bin/hiroute': cli, 'bin/hirouted': daemon, 'libexec/cliproxyapi': cpa}
        result['installed_sha256'] = {name: digest(path) for name, path in installed.items()}
        for name, path in installed.items():
            if digest(path) != package['files'][name]['sha256']:
                raise ValueError(f'installed identity differs: {name}')
            loader = checked['files'][name]['interpreter']
            if loader:
                # Eagerly resolve dependencies in the minimal image, not the build image.
                run(loader, '--list', path)
        run(cli, '--help')
        run(cpa, '--help')
        # Use the supported settings format in this disposable HOME; RUST_LOG does
        # not control the product diagnostics runtime. No user configuration is read.
        diagnostics = home / '.local/state/hiroute/diagnostics'
        diagnostics.mkdir(parents=True, mode=0o700, exist_ok=True)
        settings = diagnostics / 'settings.json'
        settings.write_text(json.dumps({'schema': 'hiroute.diagnostic-settings/v1', 'revision': 1, 'level': 'debug'}))
        settings.chmod(0o600)
        with (output / 'daemon.log').open('w') as log:
            process = subprocess.Popen([str(cli), 'service', 'run'], env=environment,
                                       stdout=log, stderr=log, start_new_session=True)
        result['pid'] = process.pid
        deadline = time.monotonic() + 60
        while True:
            if process.poll() is not None:
                raise ValueError('installed daemon exited before readiness')
            response = run(cli, 'system', 'status', '--output', 'json', required=False)
            if response.returncode == 0:
                status = json.loads(response.stdout)
                if status.get('data', {}).get('daemon') == 'role_all' and status['data'].get('gateway') == 'ready':
                    break
            if time.monotonic() >= deadline:
                raise ValueError('installed daemon never reached business readiness')
            time.sleep(0.2)
        rounds = []
        for index in range(2):
            if index:
                time.sleep(1)
                status = json.loads(run(cli, 'system', 'status', '--output', 'json').stdout)
            if (process.poll() is not None or status.get('data', {}).get('daemon') != 'role_all'
                    or status['data'].get('gateway') != 'ready'):
                raise ValueError('daemon and Gateway did not remain healthy')
            gateway = json.loads(run(cli, 'gateway', 'show', '--output', 'json').stdout)
            if gateway.get('data', {}).get('ready') is not True or not gateway['data'].get('connect_address'):
                raise ValueError('real Gateway management call did not report readiness')
            rounds.append({'system_status': status, 'gateway_show': gateway})
        result['management'] = {'rounds': rounds}
        records = [json.loads(line) for line in (diagnostics / 'daemon/current.jsonl').read_text().splitlines()]
        applied = [row for row in records if 'level_applied' in row.get('event', {})]
        if not applied or applied[-1]['event']['level_applied']['level'] != 'debug':
            raise ValueError('daemon diagnostics did not apply Debug')
        result['diagnostics'] = {'boot_id': applied[-1]['boot_id'],
                                 'level_applied': applied[-1]['event']['level_applied']}
        process.send_signal(signal.SIGTERM)
        result['daemon_exit'] = process.wait(timeout=30)
        if result['daemon_exit'] != 0:
            raise ValueError('daemon did not stop normally')
        if run(cli, 'system', 'status', '--output', 'json', required=False).returncode == 0:
            raise ValueError('management endpoint remained available after stop')
        gateway_after = run(cli, 'gateway', 'show', '--output', 'json', required=False)
        if gateway_after.returncode == 0 and json.loads(gateway_after.stdout).get('data', {}).get('ready') is True:
            raise ValueError('Gateway listener remained available after stop')
        result['status'] = 'green'
        return result
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            if process.poll() is None:
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=10)
            result['process_stopped'] = True
        if diagnostics is not None and diagnostics.exists():
            for path in diagnostics.glob('daemon/*.jsonl'):
                destination = output / 'diagnostics' / path.relative_to(diagnostics)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, destination)
        if private is not None:
            private.cleanup()
            result['private_home_removed'] = not home.exists()
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('manifest', 'archive', 'abi', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    check(args.manifest, args.archive, args.abi, args.output)
