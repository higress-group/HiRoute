#!/usr/bin/env python3
"""Non-compiling tests for the SSH workbench contract."""
import importlib.util
import contextlib
import fcntl
from pathlib import Path
import signal
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch
import hashlib
import io
import json
import os
import socket
import subprocess

spec = importlib.util.spec_from_file_location("remote_rust", Path(__file__).with_name("remote-rust.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class Contract(unittest.TestCase):
    def test_checkout_recovery_requires_a_known_unexecuted_setup_blocker(self):
        first = dict(retention='always')
        latest = dict(status='blocked', blocker='console_assets', process_exit=None,
                      plan='recovery', executor_digest='driver')
        for blocker in runner.RECOVERABLE_SETUP_BLOCKERS:
            with self.subTest(blocker=blocker):
                row = dict(latest, blocker=blocker)
                self.assertEqual(runner.checkout_recovery_modes(first, row), ('continue', 'reuse'))
        for changes in (dict(blocker='checkout_busy'), dict(blocker='continuation_changed'),
                        dict(blocker='unknown'), dict(process_exit=0), dict(command_started_at=0),
                        dict(command_finished_at=1), dict(timed_out=True), dict(status='running')):
            with self.subTest(changes=changes):
                self.assertEqual(runner.checkout_recovery_modes(first, dict(latest, **changes)), ())
        self.assertEqual(runner.checkout_recovery_modes(dict(latest, retention='on-failure'), latest), ('reuse',))
        self.assertEqual(runner.checkout_recovery_modes(dict(retention='never'), latest), ())
        failed = dict(latest, status='failed', process_exit=101, command_started_at=1, command_finished_at=2)
        self.assertEqual(runner.checkout_recovery_modes(first, failed), ('continue', 'reuse'))
        self.assertEqual(runner.checkout_recovery_modes(first, dict(failed, plan=None)), ('reuse',))
        self.assertEqual(runner.checkout_recovery_modes(dict(latest, retention='on-failure'), failed), ('reuse',))
        self.assertEqual(runner.checkout_recovery_modes(first, dict(failed, timed_out=True)), ())
        self.assertEqual(runner.checkout_recovery_modes(dict(latest, retention='on-failure'),
                                                      dict(failed, timed_out=True)), ())

    def test_recovery_command_keeps_execution_scope_and_does_not_link_the_blocked_run(self):
        key = '20261024-010203-abcdef12'
        row = dict(id=key, checkout_owner=key, retention='always', status='blocked', blocker='console_assets',
                   process_exit=None, plan='recovery', executor_digest='driver', phase='final',
                   ssh_host='fixture-host', workbench='.local/share/fixture-workbench',
                   ref='refs/heads/fixture', sha='a' * 40, timeout=123, exclusive=True,
                   schedule='workspace-dag', related_runs=['20261024-010201-abcdef12'],
                   command=['cargo', 'test', '--locked', '--workspace', '--exclude', 'hiroute-desktop', '--all-features'])
        import shlex
        argv = shlex.split(runner.checkout_recovery_command(Path('/unused'), row, '/staging with spaces/dist'))
        for flag, value in (('--host', row['ssh_host']), ('--workbench', row['workbench']),
                            ('--continue-checkout', key), ('--timeout', '123'), ('--schedule', 'workspace-dag'),
                            ('--frontend-dist', '/staging with spaces/dist'), ('--phase', 'final')):
            self.assertEqual(argv[argv.index(flag) + 1], value)
        self.assertIn('--exclusive', argv)
        self.assertEqual(argv[argv.index('--related-run') + 1], row['related_runs'][0])
        self.assertEqual(argv[argv.index('--') + 1:], row['command'])
        self.assertEqual(argv.count(key), 1)

    def test_terminal_workers_export_evidence_before_default_cleanup_and_honor_explicit_retention(self):
        # Real worktrees, checkout leases and command processes; no Rust compilation.
        for mode, retention in (('success', 'never'), ('failure', 'never'), ('blocked', 'never'),
                                ('timeout', 'never'), ('export-failure', 'never'),
                                ('unknown-untracked', 'never'), ('unknown-modified', 'never'),
                                ('unknown-ignored', 'never'), ('unknown-ignored-space', 'never'),
                                ('success', 'always'), ('failure', 'on-failure')):
            with self.subTest(mode=mode, retention=retention), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                base = Path(directory).resolve()
                source, root, home = base / 'source', base / 'workbench', base / 'home'
                subprocess.run(['git', 'init', '-q', str(source)], check=True)
                (source / '.gitignore').write_text('target/\n*.private\n')
                (source / 'source.txt').write_text('tracked source')
                git = ['git', '-C', str(source)]
                subprocess.run([*git, 'add', '.'], check=True)
                subprocess.run([*git, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], check=True)
                sha = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
                ref = subprocess.check_output([*git, 'symbolic-ref', 'HEAD'], text=True).strip()
                binary = home / '.cargo/bin'
                binary.mkdir(parents=True)
                compiler = '''import json,os,pathlib,sys,time
if '--version' in sys.argv:
 print('fixture compiler');sys.exit(0)
p=pathlib.Path.cwd()/'target'
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(p)}));sys.exit(0)
for profile in ('debug','release','x86_64-pc-windows-msvc/debug','smoke/builds'):
 (p/profile).mkdir(parents=True,exist_ok=True)
 (p/profile/'binary').write_bytes(b'rebuildable compiler fixture')
(p/'e2e').mkdir()
(p/'e2e/report.json').write_text('{"observed":"fixture"}')
if MODE=='export-failure':
 (p/'e2e/link').symlink_to(pathlib.Path.cwd()/'.gitignore')
if MODE.startswith('unknown-'):
 name={'unknown-untracked':'unknown-diagnostic.txt','unknown-modified':'source.txt','unknown-ignored':'unknown.private',
       'unknown-ignored-space':' target/unique.private'}[MODE]
 (pathlib.Path.cwd()/name).parent.mkdir(parents=True,exist_ok=True)
 (pathlib.Path.cwd()/name).write_text('unique material')
(pathlib.Path(os.environ['TMPDIR'])/'diagnostic.log').write_text('private runtime evidence')
time.sleep(30 if MODE=='timeout' else 0)
bad=MODE in ('failure','export-failure') or MODE.startswith('unknown-')
print('Running tests/fixture.rs (target/debug/deps/fixture-abcdef0123456789)')
print('running 1 test')
print('test component ... '+('FAILED' if bad else 'ok'))
print('test result: '+('FAILED' if bad else 'ok')+'. '+str(int(not bad))+' passed; '+str(int(bad))+' failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
sys.exit(101 if bad else 0)
'''
                for name in ('cargo', 'rustc'):
                    path = binary / name
                    path.write_text('#!' + sys.executable + '\nMODE=' + repr(mode) + '\n' + compiler)
                    path.chmod(0o700)
                key = '20261024-050001-abcdef12'
                run = root / 'runs' / key
                run.mkdir(parents=True)
                request = dict(id=key, ref=ref, sha=sha, executor_sha=sha, origin=str(source), retention=retention,
                               command=['cargo', 'test', '--test', 'publication_process', 'installed_'] if mode == 'blocked'
                               else ['cargo', 'test', '--lib'], timeout=1 if mode == 'timeout' else 10,
                               exclusive=False, ssh_host='fixture-host')
                runner.save(run / 'request.json', request)
                with patch.object(runner.Path, 'home', return_value=home), patch.dict(os.environ, {}, clear=False), \
                     patch.object(runner.sys, 'platform', 'linux'), \
                     patch.object(runner.tempfile, 'gettempdir', return_value=str(base)), \
                     patch.object(runner, 'MIN_FREE_BYTES', 0), \
                     patch.object(runner, 'admit_cache_server', return_value={'state': 'ready'}), \
                     patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}):
                    runner.worker(run / 'request.json')
                result = json.loads((run / 'result.json').read_text())
                self.assertEqual(result['status'], {'success': 'completed', 'failure': 'failed',
                                 'timeout': 'failed', 'blocked': 'blocked', 'export-failure': 'failed',
                                 'unknown-untracked': 'failed', 'unknown-modified': 'failed', 'unknown-ignored': 'failed',
                                 'unknown-ignored-space': 'failed'}[mode], result.get('error'))
                self.assertEqual(result['process_exit'], {'success': 0, 'failure': 101,
                                 'timeout': -9, 'blocked': None, 'export-failure': 101,
                                 'unknown-untracked': 101, 'unknown-modified': 101, 'unknown-ignored': 101,
                                 'unknown-ignored-space': 101}[mode])
                if mode.startswith('unknown-'):
                    name = {'unknown-untracked': 'unknown-diagnostic.txt', 'unknown-modified': 'source.txt',
                            'unknown-ignored': 'unknown.private', 'unknown-ignored-space': ' target/unique.private'}[mode]
                    self.assertTrue(Path(result['checkout']).is_dir(), result.get('cleanup'))
                    self.assertEqual((Path(result['checkout']) / name).read_text(), 'unique material')
                    self.assertEqual(result['cleanup']['checkout']['state'], 'retained')
                    self.assertIn('retained', result['cleanup']['checkout']['error'])
                    self.assertTrue(Path(result['temporary_directory']['path']).is_dir())
                    self.assertEqual(result['cleanup']['validation_ref']['state'], 'retained')
                    continue
                if mode == 'export-failure':
                    self.assertEqual(result['evidence_export']['state'], 'failed')
                    self.assertIn('link', result['evidence_export']['error'])
                    self.assertTrue(Path(result['checkout']).is_dir())
                    self.assertTrue(Path(result['temporary_directory']['path']).is_dir())
                    self.assertTrue((source / '.gitignore').is_file())
                    (Path(result['checkout']) / 'target/e2e/link').unlink()
                    with patch.object(runner, 'worker_alive', return_value=False):
                        cleanup = runner.cleanup_terminal_run(root, key)
                    self.assertEqual(cleanup['checkout']['state'], 'removed')
                    self.assertFalse(Path(result['temporary_directory']['path']).exists())
                    saved = json.loads((run / 'result.json').read_text())
                    self.assertEqual((saved['status'], saved['process_exit']), ('failed', 101))
                    self.assertEqual(saved['evidence_export']['state'], 'failed')
                    self.assertEqual(saved['cleanup']['evidence_export']['state'], 'saved')
                    manifest = Path(saved['cleanup']['evidence_export']['manifest'])
                    self.assertEqual((manifest.parent / 'artifacts/e2e/report.json').read_text(), '{"observed":"fixture"}')
                    continue
                if mode != 'blocked':
                    self.assertTrue((run / 'artifacts/e2e/report.json').is_file())
                self.assertEqual(result['evidence_export']['state'], 'saved')
                self.assertEqual(Path(result['checkout']).exists(), retention != 'never')
                self.assertTrue((run / 'validation-report.json').is_file())
                self.assertTrue(source.is_dir())
                if mode == 'blocked':
                    self.assertNotIn('command_started_at', result)
                    recovery = result['console_assets']['preparation'][-1]
                    self.assertIn('--retention never', recovery)
                    self.assertNotIn('--reuse-checkout', recovery)
                else:
                    self.assertEqual(Path(result['temporary_directory']['path']).exists(), retention != 'never')
                    self.assertEqual((run / 'artifacts/e2e/report.json').read_text(), '{"observed":"fixture"}')
                    self.assertEqual((run / 'evidence/temporary/diagnostic.log').read_text(), 'private runtime evidence')
                    for path in ('debug', 'release', 'x86_64-pc-windows-msvc/debug', 'smoke/builds/binary'):
                        self.assertFalse((run / 'artifacts' / path).exists())
                if retention == 'never':
                    self.assertEqual(result['cleanup']['checkout']['state'], 'removed')
                    self.assertFalse((run / 'cleanup-evidence').exists())
                    self.assertNotIn(key, subprocess.check_output(
                        ['git', '--git-dir', str(root / 'repository.git'), 'worktree', 'list', '--porcelain'], text=True))

    def test_reusing_a_retained_checkout_with_another_command_does_not_export_previous_profiles(self):
        # Two production worker calls and real Git/leases; compiler outputs are fixtures.
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            base = Path(directory).resolve()
            source, root, home = base / 'source', base / 'state', base / 'home'
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            (source / '.gitignore').write_text('target/\n')
            git = ['git', '-C', str(source)]
            subprocess.run([*git, 'add', '.'], check=True)
            subprocess.run([*git, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], check=True)
            sha = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
            ref = subprocess.check_output([*git, 'symbolic-ref', 'HEAD'], text=True).strip()
            binary = home / '.cargo/bin'
            binary.mkdir(parents=True)
            compiler = '''import json,pathlib,sys
if '--version' in sys.argv:
 print('fixture compiler');sys.exit(0)
p=pathlib.Path.cwd()/'target'
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(p)}));sys.exit(0)
profile=sys.argv[sys.argv.index('--profile')+1] if '--profile' in sys.argv else 'debug'
p.mkdir(exist_ok=True)
(p/'.rustc_info.json').write_text(json.dumps(sys.argv[1:]))
folder=p/profile
folder.mkdir(parents=True,exist_ok=True)
(folder/'.cargo-lock').touch()
(folder/'binary').write_bytes(b'rebuildable custom compiler output')
(p/'e2e/debug').mkdir(parents=True,exist_ok=True)
(p/'e2e/debug/diagnostic.log').write_text('unique runtime evidence')
'''
            for name in ('cargo', 'rustc'):
                path = binary / name
                path.write_text('#!' + sys.executable + '\n' + compiler)
                path.chmod(0o700)
            first = '20261024-190001-abcdef01'
            with patch.object(runner.Path, 'home', return_value=home), patch.dict(os.environ, {}, clear=False), \
                 patch.object(runner.sys, 'platform', 'linux'), \
                 patch.object(runner.tempfile, 'gettempdir', return_value=str(base)), \
                 patch.object(runner, 'MIN_FREE_BYTES', 0), \
                 patch.object(runner, 'admit_cache_server', return_value={'state': 'ready'}), \
                 patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}), \
                 patch.object(runner, 'worker_alive', return_value=False):
                for index, command in enumerate((['cargo', 'check', '--profile', 'ci'], ['cargo', 'check'])):
                    key = first if index == 0 else '20261024-190002-abcdef02'
                    run = root / 'runs' / key
                    run.mkdir(parents=True)
                    request = dict(id=key, ref=ref, sha=sha, executor_sha=sha, origin=str(source),
                                   retention='always' if index == 0 else 'never', command=command,
                                   timeout=10, exclusive=False, ssh_host='fixture-host')
                    if index:
                        request['reuse_checkout'] = first
                    runner.save(run / 'request.json', request)
                    runner.worker(run / 'request.json')
                    result = json.loads((run / 'result.json').read_text())
                    self.assertEqual((result['status'], result['process_exit']), ('completed', 0), result.get('error'))
                    self.assertEqual(result['checkout_owner'], first)
                    self.assertFalse((run / 'artifacts/ci').exists())
                    self.assertFalse((run / 'artifacts/debug').exists())
                    self.assertEqual((run / 'artifacts/e2e/debug/diagnostic.log').read_text(), 'unique runtime evidence')
                    self.assertTrue((root / 'worktrees' / first / 'target/ci/binary').is_file())
                    if not index:
                        original_manifest = (run / 'evidence.json').read_bytes()
                    if index:
                        self.assertEqual(result['cleanup']['checkout']['state'], 'retained')
                        self.assertFalse(Path(result['temporary_directory']['path']).exists())
                cleanup = runner.cleanup_terminal_run(root, first)
                self.assertEqual(cleanup['state'], 'removed')
                self.assertFalse((root / 'worktrees' / first).exists())
                self.assertEqual((root / 'runs' / first / 'evidence.json').read_bytes(), original_manifest)
                snapshot = Path(cleanup['evidence_export']['manifest'])
                self.assertEqual(json.loads(snapshot.read_text())['run_id'], key)
                self.assertEqual(json.loads((snapshot.parent / 'artifacts/.rustc_info.json').read_text()), ['check'])

    def test_separate_workbench_submission_readback_and_maintenance_keep_the_selected_root(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / 'remote-rust.json'
            config.write_text(json.dumps({'host': 'fixture-host', 'workbench': '.local/share/github-workbench'}))
            key = '20260905-010203-abcdef12'
            for action in ('status', 'logs', 'wait', 'cleanup', 'gc'):
                arguments = [action] + ([] if action == 'gc' else [key])
                with self.subTest(action=action), patch.object(runner, 'CONFIG', config), \
                     patch.object(runner, 'command', return_value='a' * 40), \
                     patch.object(runner.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as ssh, \
                     patch.object(runner.sys, 'argv', ['remote-rust.py', *arguments]):
                    runner.main()
                if action in ('cleanup', 'gc'):
                    payload = json.loads(ssh.call_args.kwargs['input'])
                    self.assertEqual(payload['workbench'], '.local/share/github-workbench')
                    self.assertEqual(payload['driver_args'][-1], '.local/share/github-workbench')
                    self.assertEqual(payload['report_digest'], hashlib.sha256(payload['report_source'].encode()).hexdigest())
                else:
                    self.assertIn('github-workbench', ssh.call_args.args[0][-1])
                with patch.object(runner, 'CONFIG', config), \
                     patch.object(runner.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as ssh, \
                     patch.object(runner.sys, 'argv', ['remote-rust.py', '--workbench', runner.ROOT, 'status', key]):
                    runner.main()
                self.assertIn(runner.ROOT + '/runs/', ssh.call_args.args[0][-1])
            executor, candidate = 'a' * 40, 'b' * 40
            commands = []
            def committed_command(arguments):
                commands.append(arguments)
                if arguments == ['git', 'rev-parse', 'HEAD']:
                    return executor
                if arguments == ['git', 'remote', 'get-url', 'origin']:
                    return 'fixture-origin'
                if arguments[:2] == ['git', 'show'] and arguments[2].startswith(executor + ':scripts/'):
                    return '# committed fixture source'
                self.fail('Unexpected controller command')
            with patch.object(runner, 'CONFIG', config), \
                 patch.object(runner, 'validate'), patch.object(runner, 'command', side_effect=committed_command), \
                 patch.object(runner, 'console_assets', side_effect=AssertionError('Caller must not read frontend bytes')), \
                 patch.object(runner.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as ssh, \
                 patch.object(runner.sys, 'argv', ['remote-rust.py', 'run', '--ref', 'refs/heads/fixture',
                                                  '--sha', candidate, '--plan', 'workbench-port', '--phase', 'focused',
                                                  '--frontend-dist', '~/.cache/console candidate/dist',
                                                  '--', 'cargo', 'check', '--locked']):
                runner.main()
            payload = json.loads(ssh.call_args.kwargs['input'])
            self.assertEqual((payload['sha'], payload['executor_sha']), (candidate, executor))
            self.assertEqual((payload['workbench'], payload['capacity_root']),
                             ('.local/share/github-workbench', runner.ROOT))
            self.assertIn(['git', 'show', executor + ':scripts/remote-rust.py'], commands)
            self.assertEqual(payload['source'], '# committed fixture source\n')
            self.assertEqual(payload['executor_digest'], hashlib.sha256(payload['source'].encode()).hexdigest())
            self.assertEqual(payload['frontend_dist'], '~/.cache/console candidate/dist')
            self.assertEqual(payload['ssh_host'], 'fixture-host')
            self.assertNotIn('console_assets_bundle', payload)

    def test_configured_bootstrap_keeps_repository_files_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            source = '# immutable fixture driver\n'
            payload = dict(source=source, executor_sha='a' * 40, sha='b' * 40,
                           executor_digest=hashlib.sha256(source.encode()).hexdigest(),
                           id='run-1', workbench='.local/share/github-workbench',
                           frontend_dist='~/remote-only/dist')
            with patch('pathlib.Path.home', return_value=Path(directory)), \
                 patch('sys.stdin', io.StringIO(json.dumps(payload))), patch('sys.stdout', io.StringIO()), \
                 patch('os.umask'), patch('subprocess.Popen'):
                exec(runner.BOOTSTRAP, {})
            run = Path(directory) / payload['workbench'] / 'runs/run-1'
            saved = json.loads((run / 'request.json').read_text())
            self.assertEqual(saved['frontend_dist'], '~/remote-only/dist')
            self.assertEqual(saved['workbench'], payload['workbench'])
            self.assertFalse((run / 'console-assets.json').exists())
            self.assertFalse((Path(directory) / runner.ROOT / 'runs').exists())

    def test_new_repository_worker_respects_an_existing_workbench_capacity_lease(self):
        with tempfile.TemporaryDirectory() as directory:
            account = Path(directory)
            old = account / runner.ROOT
            old.mkdir(parents=True)
            new = account / '.local/share/github-workbench'
            key = '20260905-010203-abcdef12'
            run = new / 'runs' / key
            run.mkdir(parents=True)
            request = dict(id=key, ref='refs/heads/fixture', sha='a' * 40, executor_sha='a' * 40,
                           command=['cargo', 'check'], retention='always', exclusive=False,
                           capacity_root=runner.ROOT, plan='shared-capacity', phase='focused', timeout=10)
            runner.save(run / 'request.json', request)
            actual_capacity = runner.capacity
            with actual_capacity(old, True), patch.object(runner.Path, 'home', return_value=account), \
                 patch.object(runner, 'capacity', side_effect=lambda root, exclusive: actual_capacity(root, exclusive, time.monotonic())):
                runner.worker(run / 'request.json')
            result = json.loads((run / 'result.json').read_text())
            self.assertEqual((result['status'], result['blocker']), ('blocked', 'capacity_timeout'))
            self.assertFalse((new / 'repository.git').exists())

    def test_workbench_path_rejects_shell_and_traversal_before_ssh(self):
        for bad in ('../other', '/tmp/repo', 'a/../b', 'a b', 'a;command', '$(command)', '', '.', 'a//b'):
            with self.subTest(path=bad), self.assertRaises(ValueError):
                runner.workbench_path(bad)
        self.assertEqual(runner.workbench_path('~/.local/share/github-workbench'), '.local/share/github-workbench')

    def test_eight_shared_slots_and_exclusive_build_really_exclude_each_other(self):
        import contextlib
        with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
            root = Path(directory)
            self.assertEqual((runner.WORKBENCH_SLOTS, runner.SHARED_BUILD_JOBS, runner.EXCLUSIVE_BUILD_JOBS), (8, 8, 64))
            occupied = [stack.enter_context(runner.capacity(root, False)) for _ in range(8)]
            self.assertEqual(len({handles[-1].name for handles in occupied}), 8)
            for exclusive in (False, True):
                with self.assertRaises(runner.RunBlocked):
                    with runner.capacity(root, exclusive, time.monotonic()):
                        self.fail('Capacity overcommitted')
            stack.close()
            with runner.capacity(root, True) as handles:
                self.assertEqual(len(handles), 11)
                with self.assertRaises(runner.RunBlocked):
                    with runner.capacity(root, False, time.monotonic()):
                        self.fail('Shared job entered an exclusive build')

    def test_new_capacity_waits_for_legacy_workers_and_blocks_old_exclusive_workers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (root / 'slot-1.lock').open('a') as old:
                fcntl.flock(old, fcntl.LOCK_EX)
                with self.assertRaises(runner.RunBlocked):
                    with runner.capacity(root, False, time.monotonic()):
                        self.fail('New policy bypassed old worker')
            with runner.capacity(root, False):
                for index in range(3):
                    with (root / ('slot-%s.lock' % index)).open('a') as old:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(old, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_command_monitor_retains_lock_without_leaking_it_to_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lock_path = root / 'validation.lock'
            ready = root / 'ready'
            release = root / 'release'
            code = '''import os,sys,time
descriptor=int(sys.argv[1]); lock,ready,release=sys.argv[2:]
try:
 inherited=os.fstat(descriptor).st_ino==os.stat(lock).st_ino
except OSError:
 inherited=False
open(ready,'w').write('leaked' if inherited else 'clean')
while not os.path.exists(release): time.sleep(0.01)
'''
            with lock_path.open('a') as handle:
                fcntl.flock(handle, fcntl.LOCK_EX)
                # The old direct pass_fds call exposes this descriptor to Cargo.
                legacy = subprocess.run(
                    [runner.sys.executable, '-c',
                     'import os,sys; print(os.fstat(int(sys.argv[1])).st_ino)',
                     str(handle.fileno())],
                    pass_fds=(handle.fileno(),), capture_output=True, text=True, check=True)
                self.assertEqual(int(legacy.stdout), lock_path.stat().st_ino)
                monitor = runner.start_locked_command(
                    [runner.sys.executable, '-c', code, str(handle.fileno()),
                     str(lock_path), str(ready), str(release)], root, None,
                    subprocess.DEVNULL, [handle])
                try:
                    deadline = time.monotonic() + 5
                    while not ready.exists() and monitor.poll() is None and time.monotonic() < deadline:
                        time.sleep(0.01)
                    self.assertEqual(ready.read_text(), 'clean')
                except BaseException:
                    os.killpg(monitor.pid, signal.SIGKILL)
                    monitor.wait()
                    raise
            try:
                with lock_path.open('a') as probe:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    release.touch()
                    self.assertEqual(monitor.wait(timeout=5), 0)
                    fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                if monitor.poll() is None:
                    os.killpg(monitor.pid, signal.SIGKILL)
                    monitor.wait()

    def test_worker_records_linked_focused_and_final_feedback(self):
        # Exercise real git checkouts/locks/subprocesses with a fake compiler, no SSH/SUT.
        with tempfile.TemporaryDirectory(dir='/tmp') as folder:
            base = Path(folder)
            source = base / 'source'
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            git = ['git', '-C', str(source)]
            subprocess.run([*git, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                            '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'fixture'], check=True)
            sha = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
            ref = subprocess.check_output([*git, 'symbolic-ref', 'HEAD'], text=True).strip()
            workbench = base / 'workbench'
            home = base / 'home'
            binary = home / '.cargo/bin'
            binary.mkdir(parents=True)
            fake = '''import sys,json,pathlib
if '--version' in sys.argv:
 print(pathlib.Path(sys.argv[0]).name+' fixture');sys.exit(0)
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(pathlib.Path.cwd()/'target')}));sys.exit(0)
bad='--workspace' in sys.argv
print('Running tests/runtime.rs (target/debug/deps/runtime-abcdef0123456789)')
print('running 1 test')
print('test regression ... '+('FAILED' if bad else 'ok'))
print('test result: '+('FAILED' if bad else 'ok')+'. '+str(int(not bad))+' passed; '+str(int(bad))+' failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
sys.exit(101 if bad else 0)
'''
            for name in ('cargo', 'rustc'):
                path = binary / name
                path.write_text('#!' + runner.sys.executable + '\n' + fake)
                path.chmod(0o700)
            previous = None
            for index, phase in enumerate(('focused', 'final')):
                key = '20260916-00000' + str(index) + '-abcdef12'
                run = workbench / 'runs' / key
                run.mkdir(parents=True)
                command = ['cargo', 'test', '-p', 'hiroute-e2e'] if index == 0 else ['cargo', 'test', '--workspace', '--exclude', 'hiroute-desktop']
                request = dict(id=key, ref=ref, sha=sha, executor_sha=sha, origin=str(source),
                               command=command, retention='always', exclusive=bool(index),
                               timeout=10, plan='worker-feedback', phase=phase, related_runs=[previous] if previous else [])
                if previous:
                    request['reuse_checkout'] = previous
                path = run / 'request.json'
                runner.save(path, request)
                with patch.object(runner.Path, 'home', return_value=home), patch.dict(os.environ, {}, clear=False), \
                     patch.object(runner.tempfile, 'gettempdir', return_value=str(base.resolve())), \
                     patch.object(runner, 'console_assets') as console, \
                     patch.object(runner, 'MIN_FREE_BYTES', 0), \
                     patch.object(runner, 'worker_alive', return_value=False), \
                     patch.object(runner, 'admit_cache_server', return_value={'state': 'ready'}), \
                     patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}):
                    console.return_value.preflight.return_value = dict(state='not_required', cases=[])
                    runner.worker(path)
                result = json.loads((run / 'result.json').read_text())
                if result.get('temporary_directory'):
                    runner.cleanup_temp_directory(result['temporary_directory'])
                self.assertEqual(result['process_exit'], 101 if index else 0, result)
                if previous:
                    self.assertEqual(result['checkout_owner'], previous)
                    self.assertEqual(result['checkout'], str(workbench / 'worktrees' / previous))
                self.assertTrue((run / 'validation-report.md').exists())
                previous = key
            finding = result['validation_report']['findings'][0]
            self.assertEqual(finding['kind'], 'focused_full_gap')
            self.assertEqual(finding['focused_observation'], 'ok')
            self.assertIn('package_scope', finding['context_differences'])

    def test_submission_preserves_default_and_explicit_failure_policy_with_exclusive_workspace(self):
        for extra in ([], ['--no-fail-fast']):
            command = ['cargo', 'test', '--workspace', '--exclude', 'hiroute-desktop', *extra,
                       '--', '--exact', 'case']
            def git(argv):
                if argv[:2] == ['git', 'rev-parse']:
                    return 'a' * 40
                if argv[:2] == ['git', 'show']:
                    return '# committed executor fixture'
                if argv[:2] == ['git', 'remote']:
                    return 'fixture-origin'
                raise AssertionError(argv)
            with self.subTest(extra=extra), patch.object(runner, 'command', side_effect=git), \
                 patch.object(runner.subprocess, 'run') as submit, \
                 patch.object(runner.sys, 'argv', ['remote-rust.py', '--host', 'fixture-host', 'run',
                                                 '--ref', 'refs/heads/fixture', '--', *command]):
                runner.main()
                payload = json.loads(submit.call_args.kwargs['input'])
            self.assertEqual(payload['command'], command)
            self.assertTrue(payload['exclusive'])
            self.assertEqual(payload['retention'], 'never')

    def test_bootstrap_preserves_report_helper_for_immutable_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            source, helper = '# driver\n', '# report\n'
            payload = dict(source=source, report_source=helper, executor_sha='a' * 40, sha='b' * 40,
                           executor_digest=hashlib.sha256(source.encode()).hexdigest(),
                           report_digest=hashlib.sha256(helper.encode()).hexdigest(), id='run-1')
            with patch('pathlib.Path.home', return_value=Path(directory)), \
                 patch('sys.stdin', io.StringIO(json.dumps(payload))), patch('sys.stdout', io.StringIO()), \
                 patch('os.umask'), patch('subprocess.Popen'):
                exec(runner.BOOTSTRAP, {})
            root = Path(directory) / runner.ROOT
            helper_path = root / 'controllers' / ('a' * 40 + '-report.py')
            self.assertEqual(helper_path.read_text(), helper)
            request = json.loads((root / 'runs/run-1/request.json').read_text())
            self.assertNotIn('report_source', request)
            payload.update(id='run-2', report_source='# replaced',
                           report_digest=hashlib.sha256(b'# replaced').hexdigest())
            with patch('pathlib.Path.home', return_value=Path(directory)), \
                 patch('sys.stdin', io.StringIO(json.dumps(payload))), patch('os.umask'), \
                 self.assertRaisesRegex(RuntimeError, 'immutable report helper mismatch'):
                exec(runner.BOOTSTRAP, {})

    def test_cache_admission_requires_real_output_and_retains_failure(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent)
            env = runner.cache_server_environment(root)
            for failure in (False, True):
                calls = []
                def execute(argv, checkout, environment, output, handles):
                    calls.append(argv)
                    self.assertEqual(environment, env)
                    self.assertEqual(handles, [])
                    if not failure:
                        (Path(argv[-1]) / 'libhiroute_cache_probe.rlib').write_bytes(b'library')
                    return Mock(wait=Mock(return_value=0), poll=Mock(return_value=0))
                with patch.object(runner, 'start_locked_command', side_effect=execute):
                    if failure:
                        with self.assertRaisesRegex(runner.RunBlocked, 'not stopped'):
                            runner.admit_cache_server(root, env, handles=[])
                    else:
                        self.assertEqual(runner.admit_cache_server(root, env, handles=[])['state'], 'ready')
                self.assertEqual(len(calls), 1)
                self.assertFalse(any('--stop-server' in call for call in calls))
                record = json.loads((root / 'cache-admission.json').read_text())
                self.assertEqual(record['state'], 'blocked' if failure else 'ready')
                self.assertTrue((root / 'cache-probe.log').exists())
                self.assertEqual(list(Path(env['TMPDIR']).iterdir()), [])

    def test_cache_admission_bounds_probe_timeout(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent)
            env = runner.cache_server_environment(root)
            process = Mock(pid=123, wait=Mock(side_effect=[subprocess.TimeoutExpired('sccache', 30), -9]),
                           poll=Mock(return_value=None))
            with patch.object(runner, 'start_locked_command', return_value=process), \
                 patch.object(runner.os, 'killpg') as kill, \
                 self.assertRaises(runner.RunBlocked):
                runner.admit_cache_server(root, env, handles=[])
            self.assertEqual(process.wait.call_args_list[0].kwargs, {'timeout': 30})
            self.assertEqual(process.wait.call_count, 2)
            kill.assert_called_once_with(123, signal.SIGKILL)
            self.assertEqual(json.loads((root / 'cache-admission.json').read_text())['state'], 'blocked')

    def test_cache_admission_retains_capacity_after_runner_crash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'bin'
            binary.mkdir()
            probe = binary / 'sccache'
            probe.write_text('#!' + sys.executable + '''
import json,os,sys,time
from pathlib import Path
root=Path(os.environ['PROBE_ROOT'])
try:
 inherited=os.fstat(int(os.environ['LOCK_FD'])).st_ino==(root/'lock').stat().st_ino
except OSError:
 inherited=False
(root/'ready').write_text(json.dumps({'pid':os.getpid(),'inherited':inherited}))
while not (root/'release').exists(): time.sleep(.01)
(Path(sys.argv[-1])/'libhiroute_cache_probe.rlib').write_bytes(b'library')
''')
            probe.chmod(0o700)
            code = '''import fcntl,importlib.util,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('runner',sys.argv[1])
runner=importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)
root=Path(sys.argv[2]); handle=(root/'lock').open('a'); fcntl.flock(handle,fcntl.LOCK_EX)
env=runner.cache_server_environment(root); env['LOCK_FD']=str(handle.fileno())
runner.admit_cache_server(root,env,handles=[handle])
'''
            env = dict(os.environ, PATH=str(binary) + os.pathsep + os.environ['PATH'], PROBE_ROOT=str(root))
            owner = subprocess.Popen([sys.executable, '-c', code, runner.__file__, str(root)],
                                     env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 5
                while not (root / 'ready').exists() and owner.poll() is None and time.monotonic() < deadline:
                    time.sleep(.01)
                state = json.loads((root / 'ready').read_text())
                self.assertFalse(state['inherited'], 'cache processes must not retain compiler locks')
                owner.kill()
                owner.wait(timeout=5)
                os.kill(state['pid'], 0)
                with (root / 'lock').open('a') as next_owner:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(next_owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    (root / 'release').touch()
                    deadline = time.monotonic() + 5
                    while True:
                        try:
                            fcntl.flock(next_owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
                            break
                        except BlockingIOError:
                            if time.monotonic() >= deadline:
                                self.fail('probe completion did not release capacity')
                            time.sleep(.01)
            finally:
                (root / 'release').touch()
                if owner.poll() is None:
                    owner.kill()
                owner.wait(timeout=5)

    def test_cache_server_temp_outlives_runs_and_does_not_change_test_environment(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent) / "workbench"
            root.mkdir()
            run_temp = Path(parent) / "hr-disposable"
            run_temp.mkdir(mode=0o700)
            with patch.dict(os.environ, {"TMPDIR": str(run_temp)}):
                first = runner.cache_server_environment(root)
                self.assertEqual(os.environ["TMPDIR"], str(run_temp))
                self.assertNotEqual(first["TMPDIR"], str(run_temp))
                self.assertEqual(first["SCCACHE_IDLE_TIMEOUT"], "0")
                self.assertEqual(first["SCCACHE_CACHE_SIZE"], "30G")
                self.assertEqual(runner.cleanup_temp_directory({
                    "path": str(run_temp), "system_directory": parent,
                })["state"], "removed")
                second = runner.cache_server_environment(root)
            self.assertEqual(first["TMPDIR"], second["TMPDIR"])
            self.assertEqual(Path(second["TMPDIR"]).stat().st_mode & 0o7777, 0o700)
            with patch.object(runner, "command", return_value='{"max_cache_size": 32212254720}') as call:
                runner.stats(root, "cache-after", second)
            self.assertEqual(call.call_args.kwargs["env"]["TMPDIR"], second["TMPDIR"])

    def test_cache_server_temp_rejects_symlink_or_unsafe_mode(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent)
            target = root / "other"
            target.mkdir(mode=0o700)
            directory = root / "sccache-tmp"
            directory.symlink_to(target, target_is_directory=True)
            with self.assertRaises(runner.RunBlocked):
                runner.cache_server_environment(root)
            directory.unlink()
            directory.mkdir(mode=0o700)
            directory.chmod(0o755)
            with self.assertRaises(runner.RunBlocked):
                runner.cache_server_environment(root)

    def test_private_canonical_temp_is_unique_and_usable_by_child(self):
        with tempfile.TemporaryDirectory() as parent:
            base = Path(parent) / "system"
            base.mkdir()
            alias = Path(parent) / "alias"
            alias.symlink_to(base, target_is_directory=True)
            # macOS's unittest root can exceed the Linux operational limit; test that separately.
            with patch.object(runner.tempfile, "gettempdir", return_value=str(alias)), \
                 patch.object(runner, "MAX_TMPDIR_BYTES", 1024):
                first = runner.prepare_temp_directory()
                second = runner.prepare_temp_directory()
            self.assertNotEqual(first["path"], second["path"])
            path = Path(first["path"])
            self.assertEqual(path.parent, base.resolve())
            self.assertEqual(path.stat().st_mode & 0o7777, 0o700)
            self.assertEqual(path.stat().st_uid, os.geteuid())
            env = dict(os.environ, TMPDIR=str(path))
            child = subprocess.run([runner.sys.executable, "-c",
                                    "import tempfile; print(tempfile.gettempdir()); raise SystemExit(9)"],
                                   env=env, stdout=subprocess.PIPE, text=True)
            self.assertEqual(child.returncode, 9)
            self.assertEqual(child.stdout.strip(), str(path))
            self.assertTrue(path.is_dir())  # A failed child does not discard diagnosis artifacts.

    def test_long_temp_parent_is_rejected_before_allocation(self):
        with tempfile.TemporaryDirectory() as parent:
            base = Path(parent) / ("long" * 12)
            base.mkdir()
            with patch.object(runner.tempfile, "gettempdir", return_value=str(base)):
                with self.assertRaisesRegex(RuntimeError, "too long"):
                    runner.prepare_temp_directory()
            self.assertEqual(list(base.iterdir()), [])

    def test_cleanup_temp_removes_only_a_verified_runner_directory(self):
        with tempfile.TemporaryDirectory() as parent:
            system = Path(parent) / "system"
            system.mkdir()
            owned = system / "hr-owned"
            owned.mkdir(mode=0o700)
            os.chmod(owned, 0o700)
            info = {"path": str(owned), "system_directory": str(system), "mode": "0700"}
            result = runner.cleanup_temp_directory(info)
            self.assertEqual(result["state"], "removed")
            self.assertFalse(owned.exists())

            unsafe = system / "not-a-runner-directory"
            unsafe.mkdir(mode=0o700)
            os.chmod(unsafe, 0o700)
            rejected = runner.cleanup_temp_directory({"path": str(unsafe), "system_directory": str(system)})
            self.assertEqual(rejected["state"], "retained")
            self.assertTrue(unsafe.exists())

    def test_retention_policy_only_auto_cleans_the_intended_terminal_results(self):
        self.assertTrue(runner.should_auto_cleanup({"status": "completed", "retention": "on-failure"}))
        self.assertFalse(runner.should_auto_cleanup({"status": "failed", "retention": "on-failure"}))
        self.assertTrue(runner.should_auto_cleanup({"status": "failed", "retention": "never"}))
        self.assertFalse(runner.should_auto_cleanup({"status": "completed", "retention": "always"}))

    def test_gc_preview_never_changes_result_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "runs" / "20260905-010203-abcdef12"
            run.mkdir(parents=True)
            result_path = run / "result.json"
            result_path.write_text(json.dumps({
                "id": "20260905-010203-abcdef12", "status": "failed",
                "retention": "on-failure", "finished_at": 0, "worker_pid": None,
            }))
            preview = runner.gc_terminal_runs(root, 1, False, False)
            self.assertEqual([item["id"] for item in preview["candidates"]], ["20260905-010203-abcdef12"])
            self.assertEqual(json.loads(result_path.read_text())["status"], "failed")

    def test_cleanup_terminal_run_removes_only_its_registered_generated_worktree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "workbench"
            source = Path(directory) / "source"
            root.mkdir()
            subprocess.run(["git", "init", str(source)], check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["git", "-C", str(source), "config", "user.name", "Test"], check=True)
            subprocess.run(["git", "-C", str(source), "config", "user.email", "test@example.invalid"], check=True)
            (source / "README").write_text("test\n")
            subprocess.run(["git", "-C", str(source), "add", "README"], check=True)
            subprocess.run(["git", "-C", str(source), "commit", "-m", "test"], check=True,
                           stdout=subprocess.DEVNULL)
            bare = root / "repository.git"
            subprocess.run(["git", "clone", "--bare", str(source), str(bare)], check=True,
                           stdout=subprocess.DEVNULL)
            run_id = "20260905-010203-abcdef12"
            checkout = root / "worktrees" / run_id
            checkout.parent.mkdir()
            subprocess.run(["git", "--git-dir", str(bare), "worktree", "add", "--detach", str(checkout), "HEAD"],
                           check=True, stdout=subprocess.DEVNULL)
            generated = checkout / "crates" / "daemon" / "tests" / "support" / "__pycache__"
            generated.mkdir(parents=True)
            (generated / "module.pyc").write_bytes(b"fixture")
            run = root / "runs" / run_id
            run.mkdir(parents=True)
            (run / "command.log").write_text("preserved evidence\n")
            runner.save(run / "result.json", {
                "id": run_id, "status": "completed", "retention": "on-failure",
                "worker_pid": None, "checkout": str(checkout), "finished_at": 0,
            })
            cleanup = runner.cleanup_terminal_run(root, run_id)
            self.assertEqual(cleanup["checkout"]["state"], "removed")
            self.assertEqual(cleanup["checkout"]["method"], "git-worktree-remove-force")
            self.assertFalse(checkout.exists())
            self.assertEqual((run / "command.log").read_text(), "preserved evidence\n")

    def test_reused_checkout_is_exact_leased_and_cleanup_remains_with_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'workbench'
            source = Path(directory) / 'source'
            root.mkdir()
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            (source / '.gitignore').write_text('target/\n')
            subprocess.run(['git', '-C', str(source), 'add', '.gitignore'], check=True)
            subprocess.run(['git', '-C', str(source), '-c', 'user.name=Fixture',
                            '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false',
                            'commit', '-qm', 'base'], check=True)
            sha = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
            bare = root / 'repository.git'
            subprocess.run(['git', 'clone', '-q', '--bare', str(source), str(bare)], check=True)
            owner = '20260920-010203-abcdef12'
            borrower = '20260920-010204-abcdef12'
            checkout = root / 'worktrees' / owner
            checkout.parent.mkdir()
            subprocess.run(['git', '--git-dir', str(bare), 'worktree', 'add', '-q', '--detach',
                            str(checkout), sha], check=True)
            generated = checkout / 'target' / 'smoke' / 'builds'
            generated.mkdir(parents=True)
            (generated / 'artifact').write_bytes(b'attested fixture')
            owner_run = root / 'runs' / owner
            owner_run.mkdir(parents=True)
            runner.save(owner_run / 'result.json', dict(id=owner, status='completed',
                retention='always', sha=sha, origin=str(source), executor_sha=sha,
                checkout=str(checkout), worker_pid=None))
            runner.save(root / ('checkout-' + owner + '.json'), dict(run_id=owner, sha=sha))
            request = dict(reuse_checkout=owner, sha=sha, origin=str(source), executor_sha=sha)
            self.assertEqual(runner.verify_reused_checkout(root, request), checkout)
            borrowed = root / 'runs' / borrower
            borrowed.mkdir()
            runner.save(borrowed / 'result.json', dict(id=borrower, status='completed',
                retention='on-failure', checkout_owner=owner, checkout=str(checkout), worker_pid=None))
            self.assertEqual(runner.cleanup_terminal_run(root, borrower)['checkout']['state'], 'retained')
            self.assertEqual((generated / 'artifact').read_bytes(), b'attested fixture')
            with self.assertRaises(runner.RunBlocked):
                runner.verify_reused_checkout(root, dict(request, sha='0' * 40))
            with self.assertRaises(runner.RunBlocked):
                runner.verify_reused_checkout(root, dict(request, executor_sha='0' * 40))
            lease = runner.checkout_lease(root, owner)
            try:
                self.assertEqual(runner.cleanup_terminal_run(root, owner)['checkout']['state'], 'retained')
            finally:
                lease.close()
            self.assertEqual(runner.cleanup_terminal_run(root, owner)['checkout']['state'], 'removed')
            self.assertFalse(checkout.exists())

    def test_continuation_preserves_target_and_old_sha_but_rejects_stale_or_dirty_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'workbench'
            source = Path(directory) / 'source'
            root.mkdir()
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            git = ['git', '-C', str(source)]
            (source / '.gitignore').write_text('target/\n')
            subprocess.run([*git, 'add', '.gitignore'], check=True)
            commit = [*git, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                      '-c', 'commit.gpgsign=false', 'commit', '-qm']
            subprocess.run([*commit, 'base'], check=True)
            base = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
            (source / 'change').write_text('new candidate')
            subprocess.run([*git, 'add', 'change'], check=True)
            subprocess.run([*commit, 'candidate'], check=True)
            candidate = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
            bare = root / 'repository.git'
            subprocess.run(['git', 'clone', '-q', '--bare', str(source), str(bare)], check=True)
            owner = '20260920-010201-abcdef12'
            next_run = '20260920-010202-abcdef12'
            checkout = root / 'worktrees' / owner
            checkout.parent.mkdir()
            subprocess.run(['git', '--git-dir', str(bare), 'worktree', 'add', '-q', '--detach', str(checkout), base], check=True)
            target = checkout / 'target'
            target.mkdir()
            (target / 'sentinel').write_bytes(b'compiler objects')
            owner_run = root / 'runs' / owner
            owner_run.mkdir(parents=True)
            common = dict(origin=str(source), ref='refs/heads/fixture', executor_digest='driver',
                          schedule_digest='scheduler', plan='integration')
            previous = dict(common, id=owner, status='failed', sha=base, retention='always',
                            checkout=str(checkout), worker_pid=None)
            runner.save(owner_run / 'result.json', previous)
            runner.save(root / ('checkout-' + owner + '.json'), dict(run_id=owner, sha=base))
            request = dict(common, id=next_run, continue_checkout=owner, sha=candidate)
            self.assertEqual(runner.continuation_owner(root, request), owner)
            (checkout / 'uncommitted').touch()
            with self.assertRaises(runner.RunBlocked):
                runner.continue_checkout(root, request, owner)
            (checkout / 'uncommitted').unlink()
            for key, value in (('plan', 'other'), ('executor_digest', 'changed')):
                with self.assertRaises(runner.RunBlocked):
                    runner.continue_checkout(root, dict(request, **{key: value}), owner)
            runner.continue_checkout(root, request, owner)
            self.assertEqual((target / 'sentinel').read_bytes(), b'compiler objects')
            self.assertEqual(subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip(), candidate)
            self.assertEqual(json.loads((owner_run / 'result.json').read_text())['sha'], base)
            with self.assertRaises(runner.RunBlocked):
                runner.continue_checkout(root, request, owner)

    @unittest.skipUnless(runner.sys.platform.startswith("linux"), "Linux socket path budget")
    def test_linux_socket_under_default_short_directory(self):
        info = runner.prepare_temp_directory()
        path = Path(info["path"])
        try:
            socket_path = path / "test-123456789" / "run" / "hiroute" / "control.sock"
            socket_path.parent.mkdir(parents=True)
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(str(socket_path))
        finally:
            runner.shutil.rmtree(path)  # Only this test's freshly allocated directory.

    def test_bootstrap_keeps_immutable_driver(self):
        with tempfile.TemporaryDirectory() as directory:
            source = "# committed driver\n"
            payload = dict(source=source, executor_sha="a" * 40, sha="b" * 40,
                           executor_digest=hashlib.sha256(source.encode()).hexdigest())
            # Both submissions use the same controller without rewriting its inode.
            inode = None
            for index in range(2):
                payload["id"] = "run-%s" % index
                with patch("pathlib.Path.home", return_value=Path(directory)), \
                     patch("sys.stdin", io.StringIO(json.dumps(payload))), \
                     patch("sys.stdout", io.StringIO()), patch("os.umask"), \
                     patch("subprocess.Popen") as launch:
                    exec(runner.BOOTSTRAP, {})
                    launch.assert_called_once()
                driver = Path(directory) / runner.ROOT / "controllers" / ("a" * 40 + ".py")
                self.assertEqual(driver.read_text(), source)
                if inode is not None:
                    self.assertEqual(driver.stat().st_ino, inode)
                inode = driver.stat().st_ino

    def test_bootstrap_dispatches_maintenance_without_creating_a_run(self):
        with tempfile.TemporaryDirectory() as directory:
            source = "# committed driver\n"
            payload = dict(
                mode="maintenance", driver_args=["_cleanup", "20260905-010203-abcdef12"],
                source=source, executor_sha="b" * 40,
                executor_digest=hashlib.sha256(source.encode()).hexdigest(),
                report_source="# matching report helper\n",
                report_digest=hashlib.sha256(b"# matching report helper\n").hexdigest(),
            )
            with patch("pathlib.Path.home", return_value=Path(directory)), \
                 patch("sys.stdin", io.StringIO(json.dumps(payload))), \
                 patch("sys.stdout", io.StringIO()), patch("os.umask"), \
                 patch("subprocess.run") as invoke:
                exec(runner.BOOTSTRAP, {})
            invoke.assert_called_once()
            command = invoke.call_args.args[0]
            self.assertEqual(command[-2:], ["_cleanup", "20260905-010203-abcdef12"])
            self.assertFalse((Path(directory) / runner.ROOT / "runs").exists())
            helper = Path(directory) / runner.ROOT / "controllers" / ("b" * 40 + "-report.py")
            self.assertEqual(helper.read_text(), payload['report_source'])

    def test_home_symlink_but_not_shared_target(self):
        with tempfile.TemporaryDirectory() as directory:
            real = Path(directory) / "real"
            real.mkdir()
            alias = Path(directory) / "alias"
            alias.symlink_to(real, target_is_directory=True)
            runner.check_target(alias, str(real / "target"))
            with self.assertRaises(RuntimeError):
                runner.check_target(alias, str(real / "shared"))
            (real / "target").symlink_to(real / "shared", target_is_directory=True)
            with self.assertRaises(RuntimeError):
                runner.check_target(alias, str(real / "target"))

    def test_command_boundary(self):
        runner.validate("refs/heads/codex/test", "a" * 40, ["cargo", "test", "--locked"])
        runner.validate("refs/heads/codex/test", "a" * 40, ["cargo", "fmt", "--all", "--", "--check"])
        for argv in (["cargo", "clean"], ["sh", "-c", "true"],
                     ["cargo", "fmt"],
                     ["cargo", "test", "--target-dir=/tmp/shared"],
                     ["cargo", "check", "--config", "build.target-dir='x'"]):
            with self.assertRaises(ValueError):
                runner.validate("refs/heads/codex/test", "a" * 40, argv)

    def test_sha_and_ref_boundary(self):
        for ref, sha in (("HEAD", "a" * 40), ("refs/heads/a..b", "a" * 40),
                         ("refs/heads/a", "main")):
            with self.assertRaises(ValueError):
                runner.validate(ref, sha, ["cargo", "test"])

    def test_zero_selected_is_not_success(self):
        result = runner.test_summary("Running tests/fixture.rs (target/debug/deps/fixture-abcdef0123456789)\n"
                                     "running 80 tests\ntest result: ok. 0 passed; 0 failed; 80 ignored", ["cargo", "test"])
        self.assertEqual(result["state"], "zero_tests_passed")

    def test_test_exit_is_not_scenario_verdict(self):
        result = runner.test_summary("Running tests/fixture.rs (target/debug/deps/fixture-abcdef0123456789)\n"
                                     "running 80 tests\ntest result: ok. 80 passed; 0 failed; 0 ignored", ["cargo", "test"])
        self.assertEqual(result["counts"]["passed"], 80)
        self.assertEqual(result["scenario_state"], "not_assessed")

    def test_scheduled_result_uses_terminal_counts_instead_of_nested_child_counts(self):
        log = ("test result: ok. 40 passed; 0 failed; 0 ignored\n"
               "test result: ok. 1 passed; 0 failed; 0 ignored\n")
        result = runner.scheduled_test_summary(
            log, ["cargo", "test"], {"complete": True,
                                    "counts": {"passed": 1, "failed": 0, "ignored": 0}})
        self.assertEqual(result["counts"], {"passed": 1, "failed": 0, "ignored": 0})
        self.assertEqual(result["state"], "rust_tests_passed")
        self.assertEqual(result["scenario_state"], "not_assessed")

    def test_compile_is_not_test(self):
        self.assertEqual(runner.test_summary("", ["cargo", "check"])["state"], "not_assessed")

    def test_focused_worker_result_and_wait_use_parent_target_counts(self):
        # Real worker, Git checkouts, command processes and compact wait; fake Cargo.
        parent = ("Running unittests src/lib.rs (target/debug/deps/parent-abcdef0123456789)\n"
                  "running 2 tests\nrunning 1 test\ntest child ... ok\n"
                  "test result: ok. 1 passed; 0 failed; 0 ignored; finished in 0.01s\n"
                  "test parent ... ok\ntest sibling ... ok\n"
                  "test result: ok. 2 passed; 0 failed; 0 ignored; finished in 0.02s\n")
        other = ("Running tests/other.rs (target/debug/deps/other-abcdef0123456789)\n"
                 "running 3 tests\ntest one ... ok\ntest two ... ok\ntest three ... ok\n"
                 "test result: ok. 3 passed; 0 failed; 0 ignored; finished in 0.01s\n")
        failed = parent.replace("test sibling ... ok", "test sibling ... FAILED").replace(
            "test result: ok. 2 passed; 0 failed", "test result: FAILED. 1 passed; 1 failed")
        interrupted = parent[:parent.index("test parent ... ok")]
        zero = ("Running tests/zero.rs (target/debug/deps/zero-abcdef0123456789)\n"
                "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; finished in 0.00s\n")
        cases = [
            ("nested", parent, 0, "completed", "rust_tests_passed", (2, 0, 0)),
            ("expected-child-failure", parent.replace("test child ... ok", "test child ... FAILED").replace(
                "test result: ok. 1 passed; 0 failed", "test result: FAILED. 0 passed; 1 failed"),
             0, "completed", "rust_tests_passed", (2, 0, 0)),
            ("failed-parent", failed, 101, "failed", "tests_failed", (1, 1, 0)),
            ("incomplete-parent", interrupted, 0, "failed", "unrecognized_test_output", (0, 0, 0)),
            ("same-sized-child", interrupted.replace("running 2 tests", "running 1 test"),
             0, "failed", "unrecognized_test_output", (0, 0, 0)),
            ("zero-selected", zero, 0, "failed", "zero_tests_passed", (0, 0, 0)),
            ("ignored-only", zero.replace("running 0 tests", "running 2 tests").replace(
                "0 ignored", "2 ignored"), 0, "failed", "zero_tests_passed", (0, 0, 2)),
            ("multiple-targets", parent + other, 0, "completed", "rust_tests_passed", (5, 0, 0)),
            ("early-failure", failed + other, 101, "failed", "tests_failed", (4, 1, 0)),
            ("later-incomplete", parent + other[:other.index("test one")],
             0, "failed", "unrecognized_test_output", (2, 0, 0)),
            ("unassigned", "test result: ok. 1 passed; 0 failed; 0 ignored\n" + parent,
             0, "failed", "unrecognized_test_output", (2, 0, 0)),
            ("repeated-identity", parent + parent, 0, "failed", "unrecognized_test_output", (0, 0, 0)),
            ("nonzero-exit", parent, 7, "failed", "rust_tests_passed", (2, 0, 0)),
        ]
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            root = Path(directory)
            source, home, workbench = root / 'source', root / 'home', root / 'workbench'
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            git = ['git', '-C', str(source)]
            subprocess.run([*git, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                            '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'fixture'], check=True)
            sha = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True).strip()
            ref = subprocess.check_output([*git, 'symbolic-ref', 'HEAD'], text=True).strip()
            binary = home / '.cargo/bin'
            binary.mkdir(parents=True)
            fake = '''import sys,json,pathlib
home=pathlib.Path(__file__).parents[2]
if '--version' in sys.argv: print('fixture tool'); sys.exit(0)
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(pathlib.Path.cwd()/'target')})); sys.exit(0)
print((home/'output.log').read_text(),end='')
sys.exit(int((home/'exit-code').read_text()))
'''
            for name in ('cargo', 'rustc'):
                path = binary / name
                path.write_text('#!' + sys.executable + '\n' + fake)
                path.chmod(0o700)
            for index, (name, log, exit_code, status, state, counts) in enumerate(cases):
                with self.subTest(case=name):
                    (home / 'output.log').write_text(log)
                    (home / 'exit-code').write_text(str(exit_code))
                    key = f'20261021-0000{index:02}-abcdef12'
                    run = workbench / 'runs' / key
                    run.mkdir(parents=True)
                    request = dict(id=key, ref=ref, sha=sha, executor_sha=sha, origin=str(source),
                                   command=['cargo', 'test', '--lib', '--no-fail-fast'], retention='always',
                                   exclusive=False, timeout=10, plan='focused-counts', phase='focused')
                    runner.save(run / 'request.json', request)
                    with patch.object(runner.Path, 'home', return_value=home), \
                         patch.dict(os.environ, {}, clear=False), contextlib.redirect_stdout(io.StringIO()), \
                         patch.object(runner.tempfile, 'gettempdir', return_value=str(root.resolve())), \
                         patch.object(runner, 'MIN_FREE_BYTES', 0), \
                         patch.object(runner, 'admit_cache_server', return_value={'state': 'ready'}), \
                         patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}):
                        runner.worker(run / 'request.json')
                    result = json.loads((run / 'result.json').read_text())
                    if result.get('temporary_directory'):
                        runner.cleanup_temp_directory(result['temporary_directory'])
                    observed = subprocess.run([sys.executable, '-c', runner.WAIT_BOOTSTRAP,
                                               key, '0', str(workbench)], capture_output=True,
                                              text=True, check=True)
                    compact = json.loads(observed.stdout)
                    self.assertEqual(result['status'], status, result)
                    self.assertEqual(result['process_exit'], exit_code)
                    self.assertEqual(result['tests']['state'], state)
                    self.assertEqual(result['tests']['counts'], dict(zip(('passed', 'failed', 'ignored'), counts)))
                    self.assertEqual(result['tests']['scenario_state'], 'not_assessed')
                    self.assertEqual(compact['status'], status)
                    self.assertEqual(compact['process_exit'], exit_code)
                    self.assertEqual(compact['tests'], {k: result['tests'][k] for k in ('state', 'counts')})
                    self.assertTrue((run / 'command.log').read_text().endswith(log))
                    if status == 'completed':
                        report = json.loads((run / 'validation-report.json').read_text())
                        self.assertTrue(report['complete'])
                        self.assertEqual(sum(m['counts']['passed'] for m in report['modules']), counts[0])

    def test_atomic_result(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            runner.save(path, {"status": "waiting"})
            runner.save(path, {"status": "completed"})
            self.assertIn("completed", path.read_text())
            self.assertFalse(path.with_suffix(".tmp").exists())

    def test_completed_dag_requires_complete_report_without_hiding_test_exit(self):
        result = {"schedule": "workspace-dag", "status": "completed", "process_exit": 0,
                  "tests": {"state": "rust_tests_passed"},
                  "validation_report": {"complete": False, "error": "report write failed"}}
        runner.require_dag_report(result)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["process_exit"], 0)
        self.assertIn("report write failed", result["error"])
        result.update(status="completed", validation_report={"complete": True})
        result.pop("error")
        runner.require_dag_report(result)
        self.assertEqual(result["status"], "completed")
        result.update(status="completed", validation_report={"complete": False})
        runner.require_dag_report(result)
        self.assertEqual(result["status"], "failed")
        result.update(status="completed", schedule="workspace-smoke")
        runner.require_dag_report(result)
        self.assertEqual(result["status"], "completed")

    def test_wait_observes_terminal_result_once_without_verbose_run_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_id = "20260920-120000-abcdef12"
            result = root / "runs" / run_id / "result.json"
            result.parent.mkdir(parents=True)
            runner.save(result, {"id": run_id, "status": "running", "sha": "a" * 40})
            process = subprocess.Popen(
                [sys.executable, "-c", runner.WAIT_BOOTSTRAP, run_id, "3", str(root)],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            try:
                time.sleep(0.1)
                runner.save(result, {"id": run_id, "status": "completed", "sha": "a" * 40,
                                     "schedule": "workspace-dag",
                                     "process_exit": 0, "execution_seconds": 140.611,
                                     "performance_goal_met": True,
                                     "validation_report": {"complete": True,
                                                           "top_modules": [{"module": "large"}]},
                                     "tests": {"state": "rust_tests_passed",
                                               "counts": {"passed": 2164, "failed": 0, "ignored": 19}},
                                     "schedule_result": {"complete": True, "targets": 86,
                                                         "executed_targets": 86,
                                                         "target_results": [{"log": "/private/large.log"}]}})
                output, error = process.communicate(timeout=4)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
            self.assertEqual((process.returncode, error), (0, ""))
            self.assertEqual(len(output.splitlines()), 1)
            summary = json.loads(output)
            self.assertEqual(summary["wait_state"], "terminal")
            self.assertEqual(summary["tests"]["counts"]["passed"], 2164)
            self.assertTrue(summary["performance_goal_met"])
            self.assertEqual(summary["validation_report"], {"complete": True})
            self.assertEqual(summary["schedule"], {"complete": True, "targets": 86,
                                                   "executed_targets": 86})
            self.assertNotIn("/private/large.log", output)

    def test_wait_exposes_incomplete_dag_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_id = "20260920-120000-abcdef12"
            result = root / "runs" / run_id / "result.json"
            result.parent.mkdir(parents=True)
            runner.save(result, {"id": run_id, "status": "failed", "sha": "a" * 40,
                                 "schedule": "workspace-dag", "process_exit": 0,
                                 "validation_report": {"complete": False,
                                                       "error": "report write failed " * 50}})
            observed = subprocess.run([sys.executable, "-c", runner.WAIT_BOOTSTRAP,
                                       run_id, "0", str(root)], capture_output=True,
                                      text=True, check=True)
            summary = json.loads(observed.stdout)
            self.assertEqual(summary["status"], "failed")
            self.assertEqual(summary["process_exit"], 0)
            self.assertFalse(summary["validation_report"]["complete"])
            self.assertEqual(len(summary["validation_report"]["error"]), 300)

    def test_wait_timeout_and_missing_run_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_id = "20260920-120000-abcdef12"
            command = [sys.executable, "-c", runner.WAIT_BOOTSTRAP, run_id, "0", str(root)]
            missing = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(missing.returncode, 2)
            self.assertEqual(json.loads(missing.stdout)["wait_state"], "missing")
            result = root / "runs" / run_id / "result.json"
            result.parent.mkdir(parents=True)
            runner.save(result, {"id": run_id, "status": "running", "sha": "a" * 40})
            pending = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(pending.returncode, 0)
            self.assertEqual(json.loads(pending.stdout), {
                "run_id": run_id, "wait_state": "timeout", "status": "running", "sha": "a" * 40})
            runner.save(result, {"id": run_id, "status": "failed", "sha": "a" * 40,
                                 "process_exit": 101, "error": "build failed " * 100})
            failed = subprocess.run(command, capture_output=True, text=True)
            summary = json.loads(failed.stdout)
            self.assertEqual((failed.returncode, summary["wait_state"], summary["process_exit"]),
                             (0, "terminal", 101))
            self.assertEqual(len(summary["error"]), 300)
            runner.save(result, {"id": run_id, "status": "completed", "sha": "a" * 40,
                                 "process_exit": 0, "performance_goal_met": True,
                                 "tests": {"state": "not_assessed",
                                           "counts": {"passed": 0, "failed": 0, "ignored": 0}}})
            format_check = json.loads(subprocess.run(command, capture_output=True,
                                                     text=True, check=True).stdout)
            self.assertNotIn("performance_goal_met", format_check)
            self.assertNotIn("tests", format_check)


class CleanupEvidence(unittest.TestCase):
    """Real Git, leases, exports and maintenance entry; no compiler is needed."""
    def setUp(self):
        directory = tempfile.TemporaryDirectory(dir='/tmp')
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.home = self.base / 'home'
        self.root = self.home / 'workbench'
        self.root.mkdir(parents=True)
        source = self.base / 'source'
        self.git('init', '-q', str(source))
        (source / '.gitignore').write_text('target/\nunknown.private\n')
        self.git('-C', str(source), 'add', '.')
        self.git('-C', str(source), 'commit', '-qm', 'base')
        self.sha = self.git('-C', str(source), 'rev-parse', 'HEAD')
        (source / 'tracked').write_text('candidate')
        self.git('-C', str(source), 'add', 'tracked')
        self.git('-C', str(source), 'commit', '-qm', 'candidate')
        self.candidate = self.git('-C', str(source), 'rev-parse', 'HEAD')
        bare = self.root / 'repository.git'
        self.git('clone', '-q', '--bare', str(source), str(bare))
        self.owner, self.borrower = '20261028-010201-abcdef01', '20261028-010202-abcdef02'
        self.checkout = self.root / 'worktrees' / self.owner
        self.checkout.parent.mkdir()
        self.git('--git-dir', str(bare), 'worktree', 'add', '-q', '--detach', str(self.checkout), self.sha)
        self.git('--git-dir', str(bare), 'update-ref', 'refs/validation/' + self.owner, self.sha)
        self.metadata = self.checkout / 'target/.rustc_info.json'
        self.metadata.parent.mkdir()
        self.metadata.write_text('owner compiler metadata')
        system = self.base / 'scratch'
        system.mkdir()
        self.scratch = system / 'hr-fixture'
        self.scratch.mkdir(mode=0o700)
        (self.scratch / 'diagnostic.log').write_text('owner failure evidence')
        self.run = self.root / 'runs' / self.owner
        self.run.mkdir(parents=True)
        self.common = dict(origin=str(source), ref='refs/heads/fixture', plan='cleanup-evidence',
                           executor_sha=self.sha, executor_digest='driver', report_digest='report')
        self.original = dict(self.common, id=self.owner, checkout_owner=self.owner,
                             sha=self.sha, status='failed', retention='always', worker_pid=None,
                             checkout=str(self.checkout), process_exit=101, scenario='expected_red',
                             command=['cargo', 'check'], finished_at=0,
                             temporary_directory=dict(path=str(self.scratch), system_directory=str(system)))
        self.original['evidence_export'] = runner.reporting().preserve_evidence(self.run, self.original)
        runner.save(self.run / 'result.json', self.original)
        runner.save(self.run / 'validation-report.json', dict(sha=self.sha, process_exit=101))
        runner.save(self.root / ('checkout-' + self.owner + '.json'), dict(run_id=self.owner, sha=self.sha))
        self.original_manifest = (self.run / 'evidence.json').read_bytes()
        self.original_report = (self.run / 'validation-report.json').read_bytes()

    def git(self, *arguments):
        return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
            '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', 'commit.gpgsign=false', *arguments], text=True, stderr=subprocess.PIPE).strip()

    def advance(self, advanced=True, changed=True):
        sha = self.candidate if advanced else self.sha
        request = dict(self.common, id=self.borrower, sha=sha)
        lease = runner.checkout_lease(self.root, self.owner)
        try:
            with runner.lock(self.root / 'repository.lock'):
                if advanced:
                    runner.continue_checkout(self.root, dict(request, continue_checkout=self.owner), self.owner)
                else:
                    runner.verify_reused_checkout(self.root, dict(request, reuse_checkout=self.owner))
                runner.save(self.root / ('checkout-' + self.owner + '.json'), dict(run_id=self.borrower, sha=sha))
            if changed:
                self.metadata.write_text('borrower compiler metadata')
            later = dict(self.original, id=self.borrower, sha=sha, command=['cargo', 'test'])
            later.pop('temporary_directory')
            run = self.root / 'runs' / self.borrower
            run.mkdir()
            later['evidence_export'] = runner.reporting().preserve_evidence(run, later)
            runner.save(run / 'result.json', later)
            self.borrower_bytes = (run / 'result.json').read_bytes()
            self.borrower_manifest = (run / 'evidence.json').read_bytes()
        finally:
            lease.close()

    def invoke(self, *arguments):
        bootstrap = ('import pathlib,runpy,sys; script,root,*args=sys.argv[1:]; '
                     'pathlib.Path.home=classmethod(lambda cls:pathlib.Path(root)); '
                     'sys.argv=[script,*args]; runpy.run_path(script,run_name="__main__")')
        return subprocess.run([sys.executable, '-c', bootstrap, runner.__file__, str(self.home), *arguments],
                              capture_output=True, text=True)

    def cleanup(self, expected=0):
        result = self.invoke('_cleanup', self.owner, 'workbench')
        self.assertEqual(result.returncode, expected, result.stderr or result.stdout)
        return json.loads(result.stdout)

    def assert_original_preserved(self):
        saved = json.loads((self.run / 'result.json').read_text())
        for key in ('sha', 'status', 'process_exit', 'scenario', 'evidence_export'):
            self.assertEqual(saved[key], self.original[key])
        self.assertEqual((self.run / 'evidence.json').read_bytes(), self.original_manifest)
        self.assertEqual((self.run / 'validation-report.json').read_bytes(), self.original_report)
        self.assertEqual((self.run / 'artifacts/.rustc_info.json').read_text(), 'owner compiler metadata')

    def assert_retained(self):
        cleanup = self.cleanup(expected=1)
        self.assertEqual(cleanup['state'], 'retained')
        self.assertTrue(self.checkout.is_dir())
        self.assertTrue(self.scratch.is_dir())
        self.assertEqual(cleanup['validation_ref']['state'], 'retained')
        self.assert_original_preserved()
        return cleanup

    def test_cleanup_original_source_is_idempotent_and_preserves_failure_verdict(self):
        first = self.cleanup()
        self.assertEqual(first['state'], 'removed')
        saved = first['evidence_export']
        manifest = Path(saved['manifest'])
        self.assertTrue(manifest.is_file())
        self.assertEqual(json.loads(manifest.read_text())['run_id'], self.owner)
        self.assertFalse(self.checkout.exists())
        self.assertFalse(self.scratch.exists())
        again = self.cleanup()
        self.assertEqual(again['checkout']['state'], 'absent')
        self.assertEqual(len(list((self.run / 'cleanup-evidence').iterdir())), 1)
        self.assert_original_preserved()

    def test_same_sha_reuse_retires_owner_without_modifying_either_run(self):
        self.advance(advanced=False)
        self.assert_capture_and_retirement(self.sha, 'borrower compiler metadata')

    def test_advanced_source_exports_a_separate_current_generation(self):
        self.advance()
        self.assert_capture_and_retirement(self.candidate, 'borrower compiler metadata')

    def test_advanced_source_with_unchanged_bytes_keeps_original_source_identity(self):
        self.advance(changed=False)
        self.assert_capture_and_retirement(self.candidate, 'owner compiler metadata')

    def assert_capture_and_retirement(self, sha, contents):
        cleanup = self.cleanup()
        self.assertEqual(cleanup['checkout']['state'], 'removed')
        self.assert_original_preserved()
        manifest = Path(cleanup['evidence_export']['manifest'])
        value = runner.reporting().verify_evidence(manifest.parent, cleanup['evidence_export'])
        self.assertEqual((value['source_revision'], value['run_id'], value['checkout_owner']),
                         (sha, self.borrower, self.owner))
        self.assertEqual((manifest.parent / 'artifacts/.rustc_info.json').read_text(), contents)
        self.assertEqual((manifest.parent / 'evidence/temporary/diagnostic.log').read_text(), 'owner failure evidence')
        self.assertFalse(self.checkout.exists())
        self.assertFalse(self.scratch.exists())
        self.assertEqual(cleanup['validation_ref']['state'], 'removed')
        self.assert_original_preserved()
        run = self.root / 'runs' / self.borrower
        self.assertEqual((run / 'result.json').read_bytes(), self.borrower_bytes)
        self.assertEqual((run / 'evidence.json').read_bytes(), self.borrower_manifest)

    def test_active_lease_or_latest_worker_refuses_all_owner_resource_removal(self):
        lease = runner.checkout_lease(self.root, self.owner)
        try:
            self.assert_retained()
        finally:
            lease.close()
        self.advance()
        latest = self.root / 'runs' / self.borrower / 'result.json'
        record = json.loads(latest.read_text())
        runner.save(latest, dict(record, worker_pid=os.getpid()))
        self.assert_retained()
        runner.save(latest, record)
        self.cleanup()

    def test_dirty_unknown_or_unrecorded_source_keeps_evidence_and_resources(self):
        for path in (self.checkout / 'untracked', self.checkout / 'unknown.private', self.checkout / '.gitignore'):
            with self.subTest(path=path.name):
                original = path.read_bytes() if path.exists() else None
                path.write_text('unknown source material')
                self.assert_retained()
                if original is None:
                    path.unlink()
                else:
                    path.write_bytes(original)
        self.git('-C', str(self.checkout), 'checkout', '-q', '--detach', self.candidate)
        self.assert_retained()

    def test_links_altered_exports_and_missing_manifest_refuse_cleanup_with_nonzero_exit(self):
        link = self.checkout / 'target/link'
        link.symlink_to(self.metadata)
        cleanup = self.assert_retained()
        self.assertEqual(cleanup['evidence_export']['state'], 'failed')
        link.unlink()
        exported = self.run / 'artifacts/.rustc_info.json'
        exported.write_text('tampered')
        cleanup = self.cleanup(expected=1)
        self.assertIn('differs', cleanup['checkout']['error'])
        self.assertTrue(self.checkout.exists())
        self.assertTrue(self.scratch.exists())
        exported.write_text('owner compiler metadata')
        (self.run / 'evidence.json').unlink()
        cleanup = self.cleanup(expected=1)
        self.assertIn('manifest is missing', cleanup['checkout']['error'])
        self.assertTrue(self.scratch.exists())
        (self.run / 'evidence.json').write_bytes(self.original_manifest)
        self.assert_original_preserved()

    def test_failed_removal_can_retry_without_overwriting_committed_capture(self):
        execute = runner.subprocess.run
        def refuse(arguments, **kwargs):
            if 'worktree' in arguments and 'remove' in arguments:
                raise subprocess.CalledProcessError(1, arguments)
            return execute(arguments, **kwargs)
        with patch.object(runner.subprocess, 'run', side_effect=refuse):
            first = runner.cleanup_terminal_run(self.root, self.owner)
        self.assertEqual(first['state'], 'retained')
        manifest = Path(first['evidence_export']['manifest'])
        original = manifest.read_bytes()
        self.assertTrue(self.scratch.exists())
        second = self.cleanup()
        self.assertNotEqual(first['evidence_export']['generation'], second['evidence_export']['generation'])
        self.assertEqual(manifest.read_bytes(), original)
        self.assert_original_preserved()

    def test_changed_latest_borrower_export_refuses_owner_cleanup(self):
        self.advance()
        artifact = self.root / 'runs' / self.borrower / 'artifacts/.rustc_info.json'
        artifact.write_text('changed exported borrower evidence')
        cleanup = self.assert_retained()
        self.assertIn('Previously exported evidence differs', cleanup['checkout']['error'])
        artifact.write_text('borrower compiler metadata')
        self.cleanup()

    def test_gc_refusal_is_not_reported_as_cleaned_and_propagates_nonzero(self):
        (self.checkout / 'unknown').write_text('source')
        result = self.invoke('_gc', '0', 'apply', 'include-retained', 'workbench')
        self.assertEqual(result.returncode, 1, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(len(value['candidates']), 1)
        self.assertEqual(value['cleaned'], [])
        self.assertTrue(self.checkout.exists())
        self.assertTrue(self.scratch.exists())
        self.assert_original_preserved()


if __name__ == "__main__":
    unittest.main()
