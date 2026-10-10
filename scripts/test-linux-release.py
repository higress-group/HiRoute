#!/usr/bin/env python3
"""ABI boundary regressions, including real ELF fixtures and orchestration failures."""
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, nullcontext
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


abi = load('linux-release-abi')
container = load('linux-release-container')
runtime = load('linux-release-runtime')


class AbiTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / 'hiroute'
        self.target = 'x86_64-unknown-linux-gnu'
        self.header(62)

    def header(self, machine, kind=2):
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<HH', header, 16, kind, machine)
        self.binary.write_bytes(header)

    def inspect(self, text):
        return abi.inspect(self.binary, self.target, lambda *args: text)

    def needs(self, version):
        return f'''[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]
 0x0001 (NEEDED) Shared library: [libc.so.6]
Version needs section '.gnu.version_r' contains 1 entry:
 Name: {version} Flags: none Version: 2
'''

    def test_baseline_static_and_both_native_architectures(self):
        self.assertEqual(self.inspect(self.needs('GLIBC_2.31'))['status'], 'green')
        self.assertIsNone(self.inspect('There is no dynamic section in this file.')['interpreter'])
        self.header(183)
        self.target = 'aarch64-unknown-linux-gnu'
        text = self.needs('GLIBC_2.31').replace('/lib64/ld-linux-x86-64.so.2', '/lib/ld-linux-aarch64.so.1')
        self.assertEqual(self.inspect(text)['status'], 'green')

    def test_rejects_every_reported_regression_and_other_runtime_floors(self):
        for version in ('GLIBC_2.32', 'GLIBC_2.33', 'GLIBC_2.34', 'GLIBC_2.9.99.1',
                        'GLIBC_ABI_DT_RELR', 'GLIBC_PRIVATE', 'GLIBCXX_3.4.29', 'CXXABI_1.3.13',
                        'OPENSSL_3.0.0', 'UNKNOWN_1.0'):
            if version == 'GLIBC_2.9.99.1':
                self.assertEqual(self.inspect(self.needs(version))['status'], 'green')
            else:
                with self.subTest(version=version), self.assertRaisesRegex(ValueError, version):
                    self.inspect(self.needs(version))

    def test_checks_needs_not_library_exported_definitions(self):
        self.header(62, 3)
        self.assertEqual(self.inspect("Version definition section '.gnu.version_d':\n Name: MY_API_99")['required_versions'], [])

    def test_loader_architecture_dependencies_and_new_dynamic_features(self):
        for text in (self.needs('GLIBC_2.31').replace('libc.so.6', 'libssl.so.3'),
                     self.needs('GLIBC_2.31').replace('/lib64/ld-linux-x86-64.so.2', '/tmp/custom-loader'),
                     self.needs('GLIBC_2.31') + '\n (RUNPATH) [/build/lib]',
                     self.needs('GLIBC_2.31') + '\n (RELR) 0x00'):
            with self.assertRaises(ValueError):
                self.inspect(text)
        self.header(183)
        with self.assertRaisesRegex(ValueError, 'architecture'):
            self.inspect('')

    @unittest.skipUnless(platform.system() == 'Linux' and platform.machine() == 'x86_64' and shutil.which('cc'), 'native ELF compiler required')
    def test_real_elf_baseline_library_static_executable_and_new_glibc_symbol(self):
        source = self.root / 'fixture.c'
        source.write_text('#include <stdio.h>\nvoid fixture(void) { puts("baseline"); }\n')
        subprocess.run(['cc', '-shared', '-fPIC', str(source), '-o', str(self.binary)], check=True)
        self.assertIn('GLIBC_2.2.5', abi.inspect(self.binary, self.target)['required_versions'])
        source.write_text('void _start(void) { __asm__("mov $60, %rax; xor %rdi, %rdi; syscall"); }\n')
        subprocess.run(['cc', '-nostdlib', '-static', str(source), '-o', str(self.binary)], check=True)
        self.assertEqual(abi.inspect(self.binary, self.target)['needed'], [])
        if tuple(map(int, platform.libc_ver()[1].split('.'))) <= (2, 31):
            self.skipTest('new-symbol fixture requires a newer host libc')
        source.write_text('#define _GNU_SOURCE\n#include <unistd.h>\nint fixture(void) { return close_range(100, 200, 0); }\n')
        subprocess.run(['cc', '-shared', '-fPIC', str(source), '-o', str(self.binary)], check=True)
        # The original architecture-only release check admits this failing payload.
        load('build-release').inspect_elf(self.binary, self.target)
        with self.assertRaisesRegex(ValueError, r'GLIBC_2\.34'):
            abi.inspect(self.binary, self.target)

    def test_wrong_runtime_floor_is_red_with_retained_evidence(self):
        output = self.root / 'evidence'
        with patch.object(runtime.platform, 'libc_ver', return_value=('glibc', '2.34')):
            with self.assertRaisesRegex(ValueError, 'glibc 2.31'):
                runtime.check(self.root / 'missing', self.root / 'missing', self.root / 'missing', output)
        self.assertEqual(json.loads((output / 'result.json').read_text())['status'], 'red')

    def test_no_native_container_host_never_falls_back_to_newer_libc(self):
        with patch.object(container.platform, 'machine', return_value='x86_64'), \
                patch.object(container.platform, 'system', return_value='Linux'), \
                patch.object(container.shutil, 'which', return_value=None):
            with self.assertRaisesRegex(ValueError, 'requires Docker'):
                container.run(self.target, self.root)
            with self.assertRaisesRegex(ValueError, 'native architecture'):
                container.run('aarch64-unknown-linux-gnu', self.root)

    def test_pinned_images_and_target_policy(self):
        policy = abi.baseline()
        for key in ('build_image', 'runtime_image'):
            self.assertRegex(policy[key], r'@sha256:[a-f0-9]{64}$')
        self.assertEqual(set(policy['targets']), {'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'})


@unittest.skipUnless(platform.system() == 'Linux', 'Linux runtime fixture')
class RuntimeTests(unittest.TestCase):
    """Real installer/child lifecycle fixtures; these do not prove product ABI acceptance."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.packager = load('package-standalone')
        self.target = self.packager.host_target()
        self.output = self.root / 'evidence'
        for relative in ('LICENSE', 'assets/skills/hiroute-management/SKILL.md', 'docs/standalone-cli.md'):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT.parent / relative).read_bytes())
            path.chmod(0o644)

    def package(self, case='healthy'):
        executable = self.root / 'fixture'
        executable.write_text('#!' + sys.executable + '\n' + '''
import json, os, pathlib, signal, sys, time
home = pathlib.Path(os.environ['HOME'])
ready = home / 'fixture-ready.json'
case = CASE
args = sys.argv[1:]
if args == ['--version']:
    if case == 'cli_no_version' and pathlib.Path(sys.argv[0]).name == 'hiroute':
        print(json.dumps({'error': {'code': 'UNKNOWN_COMMAND'}}))
        raise SystemExit(2)
    print('hiroute 0.2.0'); raise SystemExit(0)
if args == ['--help']:
    if case == 'daemon_no_help' and pathlib.Path(sys.argv[0]).name == 'hirouted':
        print('usage: hirouted --role all --standalone', file=sys.stderr)
        raise SystemExit(2)
    print('fixture help'); raise SystemExit(0)
if args == ['service', 'run']:
    def stop(*_):
        print('home_present_at_shutdown=' + str(home.exists()), flush=True)
        ready.unlink(missing_ok=True)
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, stop)
    diagnostics = home / '.local/state/hiroute/diagnostics'
    diagnostics.mkdir(parents=True, exist_ok=True)
    settings = diagnostics / 'settings.json'
    level = json.loads(settings.read_text())['level'] if settings.exists() else 'info'
    log = diagnostics / 'daemon/current.jsonl'
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text(json.dumps({'boot_id': 'a' * 32, 'event': {'level_applied': {'level': level, 'source': 'persisted', 'revision': 1}}}) + '\\n')
    ready.write_text('0')
    while True: time.sleep(0.02)
if not ready.exists(): raise SystemExit(6)
if args[:2] == ['system', 'status']:
    count = int(ready.read_text()) + 1; ready.write_text(str(count))
    gateway = 'unavailable' if case == 'degraded' and count > 1 else 'ready'
    print(json.dumps({'data': {'daemon': 'role_all', 'gateway': gateway}}))
elif args[:2] == ['gateway', 'show']:
    print(json.dumps({'data': {'ready': case != 'bad_gateway', 'connect_address': '127.0.0.1:50000'}}))
else: raise SystemExit(2)
'''.replace('CASE', repr(case)))
        executable.chmod(0o755)
        license_path = self.root / 'cpa.LICENSE'
        license_path.write_text('fixture license')
        license_path.chmod(0o644)
        notices = self.root / 'notices'
        notices.mkdir(exist_ok=True)
        (notices / 'THIRD-PARTY-LICENSES.txt').write_text('fixture notices')
        (notices / 'third-party-licenses.json').write_text(json.dumps({
            'schema': 'hiroute.third-party-licenses/v1', 'inputs': {},
            'packages': [{'name': 'fixture'}], 'documents': [{'name': 'fixture'}],
        }))
        for path in notices.iterdir(): path.chmod(0o644)
        args = self.packager.parser().parse_args([
            'build', '--version', '0.2.0', '--revision', 'a' * 40, '--target', self.target,
            '--hiroute', str(executable), '--hirouted', str(executable), '--cpa-binary', str(executable),
            '--cpa-version', 'fixture', '--cpa-license', str(license_path),
            '--notices', str(notices), '--output', str(self.root / 'package'),
        ])
        output = io.StringIO()
        with redirect_stdout(output), patch.object(self.packager, 'REPO', self.root):
            self.packager.build(args)
        packaged = json.loads(output.getvalue())
        self.manifest = Path(packaged['manifest'])
        self.archive = Path(packaged['archive'])
        manifest = json.loads(self.manifest.read_text())
        self.checked = {'status': 'green', 'target': self.target, 'revision': 'a' * 40,
                        'archive_sha256': runtime.digest(self.archive),
                        'manifest_sha256': runtime.digest(self.manifest), 'baseline': abi.baseline(),
                        'files': {name: {'sha256': manifest['files'][name]['sha256'], 'interpreter': None}
                                  for name in ('bin/hiroute', 'bin/hirouted', 'libexec/cliproxyapi')}}
        self.abi = self.root / 'abi.json'
        self.abi.write_text(json.dumps(self.checked))

    def check(self):
        with patch.object(runtime.platform, 'libc_ver', return_value=('glibc', '2.31')):
            return runtime.check(self.manifest, self.archive, self.abi, self.output)

    def test_installed_fixture_has_two_healthy_rounds_debug_and_orderly_stop(self):
        self.package()
        result = self.check()
        self.assertEqual(result['status'], 'green')
        self.assertEqual(len(result['management'].get('rounds', [])), 2)
        self.assertEqual(result['diagnostics']['level_applied']['level'], 'debug')
        self.assertTrue(result.get('private_home_removed', False))
        self.assertTrue(result['process_stopped'])
        self.assertIn('home_present_at_shutdown=True', (self.output / 'daemon.log').read_text())

    def test_degraded_second_round_cannot_pass_the_resident_gateway_gate(self):
        self.package('degraded')
        with self.assertRaisesRegex(ValueError, 'remain healthy'):
            self.check()
        self.assertEqual(json.loads((self.output / 'result.json').read_text())['status'], 'red')

    def test_documented_daemon_start_is_accepted_when_daemon_has_no_help_flag(self):
        self.package('daemon_no_help')
        self.assertEqual(self.check()['status'], 'green')

    def test_cli_without_version_flag_uses_verified_installation_identity(self):
        self.package('cli_no_version')
        result = self.check()
        self.assertEqual(result['status'], 'green')
        self.assertEqual(result['version'], '0.2.0')
        self.assertEqual(result['version_source'], 'verified_manifest_and_installation_marker')
        self.assertNotIn(' --version\n', (self.output / 'commands.log').read_text())

    def test_installation_version_must_match_the_verified_manifest(self):
        self.package()
        original = subprocess.run

        def tamper_after_install(args, **kwargs):
            completed = original(args, **kwargs)
            if len(args) > 1 and Path(args[1]).name == 'install-standalone.py':
                marker = Path(kwargs['env']['HOME']) / '.local/share/hiroute/standalone.json'
                value = json.loads(marker.read_text())
                value['version'] = '0.2.999'
                marker.write_text(json.dumps(value))
            return completed

        with patch.object(runtime.subprocess, 'run', side_effect=tamper_after_install):
            with self.assertRaisesRegex(ValueError, 'installed version/target'):
                self.check()

    def test_failure_stops_process_before_removing_home_and_preserves_diagnostics(self):
        self.package('bad_gateway')
        with self.assertRaisesRegex(ValueError, 'Gateway management'):
            self.check()
        result = json.loads((self.output / 'result.json').read_text())
        self.assertTrue(result['process_stopped'])
        self.assertTrue(result.get('private_home_removed', False))
        self.assertIn('home_present_at_shutdown=True', (self.output / 'daemon.log').read_text())
        self.assertTrue((self.output / 'diagnostics/daemon/current.jsonl').is_file())

    def test_abi_file_hashes_and_baseline_must_match_the_installed_payload(self):
        self.package()
        for key in ('file', 'baseline'):
            checked = json.loads(json.dumps(self.checked))
            if key == 'file': checked['files']['bin/hiroute']['sha256'] = '0' * 64
            else: checked['baseline']['minimum_glibc'] = '2.34'
            self.abi.write_text(json.dumps(checked))
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'ABI evidence'):
                self.check()


class ContainerTests(unittest.TestCase):
    """Exercise wrapper arguments and retained evidence without pretending Docker ran."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        scripts = self.root / 'scripts'
        scripts.mkdir()
        for name in ('linux-release-baseline.json', 'linux-release.Dockerfile',
                     'linux-release-runtime.py', 'install-standalone.py', 'package-standalone.py'):
            (scripts / name).write_bytes((ROOT / name).read_bytes())
        (scripts / 'local-rust.py').write_text('from contextlib import nullcontext\nclass Store:\n def locked(self, repo): return nullcontext()\n')
        self.source = self.root / 'cpa'
        self.source.mkdir()
        self.sccache = self.root / 'sccache'
        self.sccache.write_text('fixture')
        self.commands = []
        self.revision = 'a' * 40
        self.target = 'x86_64-unknown-linux-gnu'
        self.hash = 'b' * 64
        self.image_id = 'sha256:' + 'c' * 64
        self.image_config_digest = self.image_id
        self.write_build_metadata = True
        self.include_config_metadata = True
        self.attestation_sensitive = False
        self.runtime_state = 'green'
        self.addCleanup(patch.stopall)
        patch.object(container, 'REPO', self.root).start()
        patch.object(container.platform, 'system', return_value='Linux').start()
        patch.object(container.platform, 'machine', return_value='x86_64').start()
        patch.object(container.shutil, 'which', side_effect=lambda name: str(self.sccache) if name == 'sccache' else '/usr/bin/' + name).start()
        patch.object(container.subprocess, 'check_output', side_effect=self.query).start()
        patch.object(container.subprocess, 'run', side_effect=self.command).start()

    def query(self, args, **_):
        if args[:3] == ['git', 'rev-parse', 'HEAD']: return self.revision
        if args[:2] == ['git', 'status']: return ''
        if args[:3] == ['docker', 'image', 'inspect']: return self.image_id
        if args[:3] == ['go', 'env', 'GOROOT']: return str(self.root / 'go')
        if args[0] == 'git': return str(self.root / 'common.git')
        self.fail(f'unexpected query: {args}')

    def command(self, args, **_):
        self.commands.append(args)
        if args[:2] == ['docker', 'build']:
            if self.attestation_sensitive and '--provenance=false' not in args:
                self.image_id = 'sha256:' + format(len(self.commands), '064x')
            if '--metadata-file' in args and self.write_build_metadata:
                path = Path(args[args.index('--metadata-file') + 1])
                metadata = {'containerimage.digest': self.image_id}
                if self.include_config_metadata:
                    metadata['containerimage.config.digest'] = self.image_config_digest
                path.write_text(json.dumps(metadata))
        elif 'scripts/build-release.py' in args:
            pointer = next(value.split('=', 1)[1] for value in args if value.startswith('HIROUTE_RELEASE_RESULT='))
            output = Path(pointer).parent / 'package'
            assets = output / 'assets'
            assets.mkdir(parents=True)
            archive = assets / 'candidate.tar.gz'
            archive.write_bytes(b'fixture archive')
            manifest = assets / 'candidate.tar.gz.json'
            manifest.write_text('fixture manifest')
            (output / 'linux-abi.json').write_text('{}')
            package = {'status': 'awaiting_runtime', 'revision': self.revision, 'assets': str(assets),
                       'website_artifact': {'filename': archive.name, 'manifest_filename': manifest.name, 'sha256': self.hash}}
            (output / 'result.json').write_text(json.dumps(package))
            Path(pointer).write_text(json.dumps({'result_path': str(output / 'result.json')}))
        elif any(value.endswith('linux-release-runtime.py') for value in args):
            output_arg = args[args.index('--output') + 1]
            mounts = [args[i + 1] for i, value in enumerate(args) if value == '-v']
            output_mount = next(value for value in mounts if value.split(':')[1] == output_arg)
            output = Path(output_mount.split(':')[0])
            (output / 'result.json').write_text(json.dumps({'status': self.runtime_state, 'archive_sha256': self.hash}))
        return subprocess.CompletedProcess(args, 0)

    def test_minimum_runtime_only_receives_staged_inputs_and_build_jobs_are_preserved(self):
        with patch.dict(os.environ, {'CARGO_BUILD_JOBS': '8'}):
            result = container.run(self.target, self.source)
        self.assertEqual(result['status'], 'completed')
        build = next(args for args in self.commands if 'scripts/build-release.py' in args)
        self.assertIn('CARGO_BUILD_JOBS=8', build)
        runtime_args = next(args for args in self.commands if any(value.endswith('linux-release-runtime.py') for value in args))
        mounts = [runtime_args[i + 1] for i, value in enumerate(runtime_args) if value == '-v']
        self.assertEqual(len(mounts), 2)
        self.assertFalse(any(value.startswith(str(self.root) + ':') for value in mounts))
        inputs = next(Path(value.split(':')[0]) for value in mounts if value.endswith(':/input:ro'))
        self.assertEqual(set(path.name for path in inputs.iterdir()), {
            'linux-release-runtime.py', 'install-standalone.py', 'package-standalone.py',
            'linux-release-baseline.json', 'candidate.tar.gz', 'candidate.tar.gz.json', 'linux-abi.json',
        })

    def test_non_green_runtime_keeps_failed_result_and_awaiting_package(self):
        self.runtime_state = 'red'
        with self.assertRaisesRegex(ValueError, 'minimum runtime'):
            container.run(self.target, self.source)
        output = next((self.root / 'target/release-candidate').glob('**/container-*'))
        self.assertEqual(json.loads((output / 'result.json').read_text())['status'], 'failed')
        self.assertEqual(json.loads((output / 'package/result.json').read_text())['status'], 'awaiting_runtime')

    def test_missing_sccache_never_mounts_the_current_directory_as_a_binary(self):
        with patch.object(container.shutil, 'which', side_effect=lambda name: '/usr/bin/docker' if name == 'docker' else None):
            with self.assertRaisesRegex(ValueError, 'sccache'):
                container.run(self.target, self.source)

    def test_failed_image_prerequisite_retains_red_evidence_before_product_build(self):
        def failed_prerequisite(args, **_):
            self.commands.append(args)
            return subprocess.CompletedProcess(args, 100)

        with patch.object(container.subprocess, 'run', side_effect=failed_prerequisite):
            with self.assertRaises(subprocess.CalledProcessError):
                container.run(self.target, self.source)
        self.assertEqual(len(self.commands), 1)
        self.assertEqual(self.commands[0][:2], ['docker', 'build'])
        output = next((self.root / 'target/release-candidate').glob('**/container-*'))
        result = json.loads((output / 'result.json').read_text())
        self.assertEqual(result['status'], 'failed')
        self.assertIn('100', result['error'])
        self.assertEqual(result['revision'], self.revision)
        self.assertEqual(result['baseline'], abi.baseline())
        self.assertFalse((self.root / 'target/.linux-release-baseline.json').exists())

    def test_existing_proxy_names_reach_builds_without_values_or_runtime_forwarding(self):
        proxies = {'HTTPS_PROXY': 'https://fixture-proxy.invalid/private-value',
                   'http_proxy': 'http://fixture-proxy.invalid/private-value',
                   'ALL_PROXY': 'socks5://fixture-proxy.invalid/private-value',
                   'NO_PROXY': 'fixture-bypass.invalid'}
        with patch.dict(os.environ, proxies, clear=True):
            container.run(self.target, self.source)
        docker_build = self.commands[0]
        build_arguments = [docker_build[index + 1] for index, value in enumerate(docker_build)
                           if value == '--build-arg']
        build = next(args for args in self.commands if 'scripts/build-release.py' in args)
        build_environment = [build[index + 1] for index, value in enumerate(build) if value == '-e']
        runtime_arguments = next(args for args in self.commands
                                 if any(value.endswith('linux-release-runtime.py') for value in args))
        runtime_environment = [runtime_arguments[index + 1] for index, value in enumerate(runtime_arguments)
                               if value == '-e']
        for name, secret_value in proxies.items():
            with self.subTest(name=name):
                self.assertIn(name, build_arguments)
                self.assertIn(name, build_environment)
                self.assertFalse(any(value == name or value.startswith(name + '=') for value in runtime_environment))
                self.assertFalse(any(secret_value in argument for args in self.commands for argument in args))

    def test_unset_proxies_are_not_added_to_build_or_runtime_arguments(self):
        with patch.dict(os.environ, {}, clear=True):
            container.run(self.target, self.source)
        for args in self.commands:
            self.assertFalse(any(argument.lower().split('=', 1)[0] in
                                 {'http_proxy', 'https_proxy', 'ftp_proxy', 'all_proxy', 'no_proxy'}
                                 for argument in args))

    def test_builds_share_host_proxy_network_while_minimum_runtime_is_offline(self):
        with patch.dict(os.environ, {'HTTPS_PROXY': 'http://127.0.0.1:12345'}, clear=True):
            container.run(self.target, self.source)
        docker_build = self.commands[0]
        compiler = next(args for args in self.commands if 'scripts/build-release.py' in args)
        runtime = next(args for args in self.commands
                       if any(value.endswith('linux-release-runtime.py') for value in args))
        for stage, args in (('recipe', docker_build), ('compiler', compiler), ('runtime', runtime)):
            with self.subTest(stage=stage):
                networks = [args[index + 1] for index, value in enumerate(args) if value == '--network']
                self.assertEqual(networks, ['none'] if stage == 'runtime' else ['host'])
        runtime_environment = [runtime[index + 1] for index, value in enumerate(runtime) if value == '-e']
        self.assertNotIn('HTTPS_PROXY', runtime_environment)

    def test_sccache_ipc_stays_in_the_container_filesystem(self):
        with patch.dict(os.environ, {'SCCACHE_SERVER_PORT': '4226',
                                    'SCCACHE_SERVER_UDS': '/sccache/shared.sock'}):
            container.run(self.target, self.source)
        compiler = next(args for args in self.commands if 'scripts/build-release.py' in args)
        environment = [compiler[index + 1] for index, value in enumerate(compiler) if value == '-e']
        self.assertIn('SCCACHE_SERVER_UDS=/tmp/hiroute-release-sccache.sock', environment)
        self.assertFalse(any(value.split('=', 1)[0] == 'SCCACHE_SERVER_PORT' for value in environment))
        self.assertNotIn('SCCACHE_SERVER_UDS=/sccache/shared.sock', environment)
        mounts = [compiler[index + 1].split(':')[1] for index, value in enumerate(compiler) if value == '-v']
        self.assertFalse(any('/tmp/hiroute-release-sccache.sock'.startswith(path.rstrip('/') + '/')
                             for path in mounts))
        runtime = next(args for args in self.commands
                       if any(value.endswith('linux-release-runtime.py') for value in args))
        self.assertFalse(any(value.startswith('SCCACHE_') for value in runtime))

    def test_selected_sccache_mount_takes_precedence_over_cargo_home_tools(self):
        container.run(self.target, self.source)
        compiler = next(args for args in self.commands if 'scripts/build-release.py' in args)
        environment = [compiler[index + 1] for index, value in enumerate(compiler) if value == '-e']
        paths = next(value.split('=', 1)[1].split(':') for value in environment if value.startswith('PATH='))
        self.assertLess(paths.index('/usr/local/bin'), paths.index('/opt/cargo/bin'))
        mounts = [compiler[index + 1] for index, value in enumerate(compiler) if value == '-v']
        self.assertIn(str(self.sccache) + ':/usr/local/bin/sccache:ro', mounts)

    def test_disabled_attestations_preserve_stable_identity_in_both_image_stores(self):
        for store in ('classic', 'containerd'):
            with self.subTest(store=store):
                stamp = self.root / 'target/.linux-release-baseline.json'
                stamp.unlink(missing_ok=True)
                self.image_id = 'sha256:' + 'c' * 64
                self.include_config_metadata = store == 'classic'
                self.attestation_sensitive = store == 'containerd'
                first = container.run(self.target, self.source)
                second = container.run(self.target, self.source)
                self.assertEqual(second['status'], 'completed')
                self.assertEqual(first['build_image_id'], second['build_image_id'])
                self.assertTrue(all('--provenance=false' in args for args in self.commands
                                    if args[:2] == ['docker', 'build']))
                identity = json.loads(stamp.read_text())
                self.assertEqual(identity['image_digest'], self.image_id)

    def test_changed_runtime_image_is_rejected_in_both_image_stores(self):
        for store in ('classic', 'containerd'):
            with self.subTest(store=store):
                stamp = self.root / 'target/.linux-release-baseline.json'
                stamp.unlink(missing_ok=True)
                self.include_config_metadata = store == 'classic'
                self.image_id = 'sha256:' + 'c' * 64
                self.image_config_digest = self.image_id
                container.run(self.target, self.source)
                self.image_id = 'sha256:' + 'd' * 64
                if store == 'classic': self.image_config_digest = self.image_id
                with self.assertRaisesRegex(ValueError, 'different Linux build baseline'):
                    container.run(self.target, self.source)

    def test_invalid_image_digest_fails_before_product_build(self):
        for image_id in ('', 'sha256:short', 'sha256:' + 'G' * 64, 'not-a-digest'):
            with self.subTest(image_id=image_id):
                self.commands.clear()
                self.image_id = image_id
                with self.assertRaisesRegex(ValueError, 'image digest'):
                    container.run(self.target, self.source)
                self.assertEqual(len(self.commands), 1)

    def test_unpublished_old_stamp_requires_a_fresh_worktree(self):
        stamp = self.root / 'target/.linux-release-baseline.json'
        stamp.parent.mkdir()
        stamp.write_text(json.dumps({'image': self.image_id, 'target': self.target}))
        with self.assertRaisesRegex(ValueError, 'different Linux build baseline'):
            container.run(self.target, self.source)


if __name__ == '__main__':
    unittest.main()
