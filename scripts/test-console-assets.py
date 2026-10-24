#!/usr/bin/env python3
"""Console selection, provenance and real managed-checkout admission regressions."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


assets = load('console-assets')
runner = load('remote-rust')


class Selection(unittest.TestCase):
    def test_selected_installed_cases_and_broad_commands(self):
        for args in ([], ['--workspace', '--exclude', 'hiroute-desktop', '--all-features'],
                     ['-p', 'hiroute-daemon'], ['--package=hiroute-daemon'], ['-phiroute-daemon'],
                     ['-p', 'hiroute-*', '--test', 'publication_*'],
                     ['--test', 'publication_process', '--', 'installed_'],
                     ['--test', 'publication_process', '--', '--include-ignored']):
            with self.subTest(args=args):
                self.assertEqual(assets.selected(['cargo', 'test', *args], 'linux'), list(assets.CASES))
        for case in assets.CASES:
            self.assertEqual(assets.selected(['cargo', 'test', '-p', 'hiroute-daemon', '--test',
                'publication_process', case, '--', '--exact'], 'linux'), [case])
        source = Path(__file__).parent.parent / 'crates/daemon/tests/publication_process.rs'
        import re
        self.assertEqual(set(re.findall(r'fn (installed_\w+)\(', source.read_text())), set(assets.CASES))

    def test_unrelated_commands_do_not_require_or_stage_console(self):
        for command in (['cargo', 'check'], ['cargo', 'build'],
                        ['cargo', 'test', '-p', 'hiroute-cli'],
                        ['cargo', 'test', '--workspace', '--exclude=hiroute-daemon'],
                        ['cargo', 'test', '-p', 'hiroute-daemon', '--lib'],
                        ['cargo', 'test', '--test=control_process'],
                        ['cargo', 'test', '--test', 'publication_process', 'real_process_'],
                        ['cargo', 'test', '--no-run'], ['cargo', 'test', '--', '--list'],
                        ['cargo', 'test', '--', '--ignored'],
                        ['cargo', 'test', '--', '--skip', 'installed_'],
                        ['cargo', 'test', '--target', 'x86_64-pc-windows-msvc']):
            with self.subTest(command=command), patch.object(assets, 'stage') as stage:
                self.assertEqual(assets.preflight(Path('/unused'), dict(command=command),
                    'linux', '/unused/missing-dist'), dict(state='not_required', cases=[]))
                stage.assert_not_called()
        self.assertEqual(assets.selected(['cargo', 'test'], 'darwin'), [])

    def test_exact_filters_and_skips_keep_remaining_cases(self):
        first = assets.CASES[0]
        self.assertEqual(assets.selected(['cargo', 'test', first, '--', '--skip=' + first,
                                         '--exact'], 'linux'), [])
        self.assertEqual(assets.selected(['cargo', 'test', first, '--', '--skip=installed_',
                                         '--exact'], 'linux'), [first])
        self.assertEqual(assets.selected(['cargo', 'test', '--', '--skip', first], 'linux'),
                         list(assets.CASES[1:]))


class Artifact(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir='/tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / 'source'
        self.repo.mkdir()
        (self.repo / 'apps/desktop').mkdir(parents=True)
        (self.repo / 'apps/desktop/package.json').write_text('{}')
        (self.repo / '.gitignore').write_text('apps/desktop/dist/\ntarget/\n')
        self.git('init', '-q')
        self.git('add', '.')
        self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                 '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
        self.sha = self.git('rev-parse', 'HEAD')
        self.dist = self.repo / 'apps/desktop/dist'
        self.dist.mkdir()
        (self.dist / 'index.html').write_text('<script src="/assets/app.js"></script>')
        (self.dist / 'assets').mkdir()
        (self.dist / 'assets/app.js').write_text('console.log("fixture asset")')
        (self.dist / assets.RECEIPT).write_text(json.dumps(assets.manifest(self.sha, assets.files(self.dist))))

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args], text=True).strip()

    def execute_worker(self, root, row, source=None, cache_error=None):
        home = self.root / 'home'
        binary = home / '.cargo/bin'
        binary.mkdir(parents=True, exist_ok=True)
        fake = '''import sys,json,pathlib
if '--version' in sys.argv:
 print(pathlib.Path(sys.argv[0]).name+' fixture');sys.exit(0)
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(pathlib.Path.cwd()/'target')}));sys.exit(0)
bad=(pathlib.Path(sys.argv[0]).resolve().parents[2]/'fail-command').exists()
print('Running tests/publication_process.rs (target/debug/deps/publication_process-abcdef0123456789)')
print('running 1 test')
print('test component_fixture ... '+('FAILED' if bad else 'ok'))
print('test result: '+('FAILED' if bad else 'ok')+'. '+str(int(not bad))+' passed; '+str(int(bad))+' failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
sys.exit(101 if bad else 0)
'''
        for name in ('cargo', 'rustc'):
            path = binary / name
            path.write_text('#!' + sys.executable + '\n' + fake)
            path.chmod(0o700)
        run = root / 'runs' / row['id']
        run.mkdir(parents=True)
        if source is not None:
            row = dict(row, frontend_dist=str(source))
        runner.save(run / 'request.json', row)
        with patch.object(runner.Path, 'home', return_value=home), patch.dict(os.environ, {}, clear=False), \
             patch.object(runner.sys, 'platform', 'linux'), \
             patch.object(runner.tempfile, 'gettempdir', return_value=str(self.root)), \
             patch.object(runner, 'worker_alive', return_value=False), \
             patch.object(runner, 'MIN_FREE_BYTES', 0), \
             patch.object(runner, 'admit_cache_server', side_effect=cache_error, return_value={'state': 'ready'}), \
             patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}) as stats:
            runner.worker(run / 'request.json')
        result = json.loads((run / 'result.json').read_text())
        if result.get('temporary_directory'):
            self.addCleanup(runner.cleanup_temp_directory, result['temporary_directory'])
        return result, stats

    def generated_request(self, command, run_id, executor_sha=None):
        argv = shlex.split(command)
        self.assertEqual(argv[:2], ['python3', 'scripts/remote-rust.py'])
        revision = executor_sha or self.git('rev-parse', 'HEAD')
        def committed(arguments):
            if arguments == ['git', 'rev-parse', 'HEAD']:
                return revision
            if arguments == ['git', 'remote', 'get-url', 'origin']:
                return str(self.repo)
            if arguments[:2] == ['git', 'show']:
                return (Path(__file__).parent / arguments[2].split(':scripts/', 1)[1]).read_text().rstrip('\n')
            self.fail('Unexpected submission read: ' + repr(arguments))
        with patch.object(runner, 'command', side_effect=committed), \
             patch.object(runner.sys, 'argv', argv[1:]), \
             patch.object(runner.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as ssh:
            runner.main()
        request = json.loads(ssh.call_args.kwargs['input'])
        request['id'] = run_id
        return request

    def test_build_stamps_only_successful_unchanged_source(self):
        with patch.object(assets.subprocess, 'call', return_value=0) as npm:
            self.assertEqual(assets.build(self.repo, {}), 0)
        self.assertEqual(npm.call_args.args[0], ['npm', '--prefix', 'apps/desktop', 'run', 'build'])
        self.assertEqual(assets.verify(self.dist, self.sha)['source_revision'], self.sha)
        with patch.object(assets.subprocess, 'call', return_value=12):
            self.assertEqual(assets.build(self.repo, {}), 12)
        self.assertFalse((self.dist / assets.RECEIPT).exists())
        with patch.object(assets, 'source_revision', side_effect=[self.sha, '0' * 40]), \
             patch.object(assets.subprocess, 'call', return_value=0), self.assertRaisesRegex(ValueError, 'changed'):
            assets.build(self.repo, {})
        (self.repo / 'apps/desktop/package.json').write_text('{"dirty":true}')
        with self.assertRaisesRegex(ValueError, 'clean committed'):
            assets.build(self.repo, {})

    def test_staging_rejects_old_sha_wrong_hash_missing_extra_and_linked_files(self):
        receipt = assets.verify(self.dist, self.sha)
        destination = self.root / 'destination'
        (destination / 'apps/desktop').mkdir(parents=True)
        assets.stage(destination, self.sha, self.dist)
        retained = destination / 'apps/desktop/dist'
        self.assertEqual(assets.verify(retained, self.sha), receipt)
        self.assertEqual(assets.verify(self.dist, self.sha), receipt)
        for kind in ('old', 'hash', 'missing', 'extra', 'file-link', 'directory-link', 'receipt-link', 'special'):
            bad = self.root / kind
            shutil.copytree(self.dist, bad)
            if kind == 'old':
                (bad / assets.RECEIPT).write_text(json.dumps(dict(receipt, source_revision='0' * 40)))
            elif kind == 'hash':
                (bad / 'index.html').write_text('corrupted')
            elif kind == 'missing':
                (bad / 'index.html').unlink()
            elif kind == 'extra':
                (bad / 'extra').write_text('')
            elif kind in ('file-link', 'receipt-link'):
                name = assets.RECEIPT if kind == 'receipt-link' else 'index.html'
                (bad / name).unlink()
                (bad / name).symlink_to(self.dist / name)
            elif kind == 'directory-link':
                shutil.rmtree(bad / 'assets')
                (bad / 'assets').symlink_to(self.dist / 'assets', target_is_directory=True)
            elif kind == 'special':
                os.mkfifo(bad / 'pipe')
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                assets.stage(destination, self.sha, bad)
            self.assertEqual(assets.verify(retained, self.sha), receipt)
        with self.assertRaisesRegex(ValueError, 'outside'):
            assets.stage(destination, self.sha, retained)
        with patch.object(assets, 'MAX_BYTES', 1), self.assertRaisesRegex(ValueError, 'size limit'):
            assets.stage(destination, self.sha, self.dist)
        snapshots = [assets.files(self.dist), dict(assets.files(self.dist), **{'index.html': b'changed'})]
        with patch.object(assets, 'files', side_effect=snapshots), self.assertRaisesRegex(ValueError, 'changed'):
            assets.stage(destination, self.sha, self.dist)
        with self.assertRaisesRegex(ValueError, 'execution-host'):
            assets.preflight(destination, dict(command=['cargo', 'test'], sha=self.sha), 'linux', 'relative/dist')
        (destination / 'apps/desktop/dist').rename(destination / 'apps/desktop/original')
        retained.symlink_to(destination / 'apps/desktop/original', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'link'):
            assets.stage(destination, self.sha, self.dist)
        self.assertEqual(assets.verify(retained.parent / 'original', self.sha), receipt)

    def test_clean_worker_blocks_before_rust_then_reuses_same_checkout_with_verified_assets(self):
        root = self.root / 'workbench'
        first = '20261021-000001-abcdef12'
        second = '20261021-000002-abcdef12'
        ref = self.git('symbolic-ref', 'HEAD')
        request = dict(id=first, ref=ref, sha=self.sha, executor_sha=self.sha, origin=str(self.repo),
                       command=['cargo', 'test', '-p', 'hiroute-daemon', '--test', 'publication_process',
                                'installed_', '--', '--nocapture'],
                       retention='on-failure', exclusive=False, timeout=10, plan='console-assets', phase='focused',
                       ssh_host='fixture-backend')
        def execute(row, source=None):
            return self.execute_worker(root, row, source)

        before, stats = execute(request)
        self.assertEqual((before['status'], before['blocker'], before['process_exit']),
                         ('blocked', 'console_assets', None), before)
        self.assertNotIn('command_started_at', before)
        stats.assert_not_called()
        self.assertFalse((root / 'runs' / first / 'command.log').exists())
        self.assertIn('--reuse-checkout ' + first, before['console_assets']['preparation'][-1])
        staging = self.root / 'home/.cache/hiroute/console-assets' / first
        preparation = before['console_assets']['preparation']
        self.assertIn(shlex.join(['scp', '-r', 'apps/desktop/dist', 'fixture-backend:' + str(staging) + '/']), preparation)
        arguments = shlex.split(preparation[-1])
        self.assertEqual(arguments[arguments.index('--frontend-dist') + 1], str(staging / 'dist'))
        row = dict(request, id=second, reuse_checkout=first, retention='always')
        lease = runner.checkout_lease(root, first)
        try:
            blocked, _ = execute(dict(row, id='20261021-000003-abcdef12'), self.dist)
            self.assertEqual(blocked.get('blocker'), 'checkout_busy')
        finally:
            lease.close()
        for index, kind in enumerate(('sha', 'hash'), 4):
            bad = self.root / ('worker-' + kind)
            shutil.copytree(self.dist, bad)
            if kind == 'sha':
                receipt = json.loads((bad / assets.RECEIPT).read_text())
                (bad / assets.RECEIPT).write_text(json.dumps(dict(receipt, source_revision='0' * 40)))
            else:
                (bad / 'index.html').write_text('corrupted')
            invalid, stats = execute(dict(row, id='20261021-00000' + str(index) + '-abcdef12'), bad)
            self.assertEqual(invalid.get('blocker'), 'console_assets')
            self.assertNotIn('command_started_at', invalid)
            stats.assert_not_called()
        after, _ = execute(row, self.dist)
        self.assertEqual(after['status'], 'completed', after)
        self.assertEqual(after['checkout'], before['checkout'])
        self.assertEqual(after['console_assets']['state'], 'verified')
        (Path(after['checkout']) / 'apps/desktop/dist/assets/app.js').write_text('damaged retained asset')
        invalid, stats = execute(dict(row, id='20261021-000006-abcdef12'))
        self.assertEqual(invalid.get('blocker'), 'console_assets')
        stats.assert_not_called()
        self.assertEqual(json.loads((root / 'runs' / first / 'result.json').read_text())['status'], 'blocked')
        runner.cleanup_temp_directory(after['temporary_directory'])

    def test_multiple_advances_block_then_execute_the_generated_recovery_in_the_same_checkout(self):
        # Real Git/leases/worker/parser and command processes; compiler output is a fixture.
        for kind in ('missing', 'stale'):
            with self.subTest(kind=kind):
                root = self.root / ('workbench-' + kind)
                owner, middle, latest, blocked, retry, recovered = [
                    '20261024-0' + str(int(kind == 'stale')) + '000' + str(i) + '-abcdef12'
                    for i in range(1, 7)]
                sha = self.git('rev-parse', 'HEAD')
                common = dict(ref=self.git('symbolic-ref', 'HEAD'), sha=sha, executor_sha=sha,
                              origin=str(self.repo), retention='always',
                              exclusive=False, timeout=37, plan='console-recovery', phase='focused',
                              ssh_host='fixture-backend', workbench='.local/share/fixture-workbench', related_runs=[])
                for key, name in (('executor', 'remote-rust'), ('schedule', 'validation-schedule'),
                                  ('report', 'validation-report'), ('console', 'console-assets')):
                    contents = (Path(__file__).parent / (name + '.py')).read_text().rstrip('\n') + '\n'
                    common[key + '_digest'] = hashlib.sha256(contents.encode()).hexdigest()
                home = self.root / 'home'
                home.mkdir(exist_ok=True)
                (home / 'fail-command').touch()
                # The stale case leaves an admitted older receipt through the runner itself.
                (self.dist / assets.RECEIPT).write_text(json.dumps(assets.manifest(sha, assets.files(self.dist))))
                first, _ = self.execute_worker(root, dict(common, id=owner,
                    command=['cargo', 'test', '--test', 'publication_process', 'installed_'] if kind == 'stale'
                    else ['cargo', 'test', '--lib']), self.dist if kind == 'stale' else None)
                (home / 'fail-command').unlink()
                self.assertEqual(first['status'], 'failed', first)
                checkout = Path(first['checkout'])
                (checkout / 'target').mkdir(exist_ok=True)
                sentinel = checkout / 'target/retained-compiler-objects'
                sentinel.write_bytes(b'retained compilation')
                previous = owner
                history = [owner]
                for index, key in enumerate((middle, latest), 1):
                    (self.repo / 'advance').write_text(kind + str(index))
                    self.git('add', 'advance')
                    self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                             '-c', 'commit.gpgsign=false', 'commit', '-qm', 'advance fixture')
                    common['sha'] = self.git('rev-parse', 'HEAD')
                    result, _ = self.execute_worker(root, dict(common, id=key, continue_checkout=previous,
                                                              command=['cargo', 'test', '--lib']))
                    self.assertEqual(result['status'], 'completed', result)
                    previous = key
                    history.append(key)
                command = ['cargo', 'test', '-p', 'hiroute-daemon', '--test', 'publication_process', 'installed_']
                blocked_request = dict(common, id=blocked, continue_checkout=latest,
                                       phase='final', related_runs=[middle, latest], command=command)
                before, stats = self.execute_worker(root, blocked_request)
                self.assertEqual((before['status'], before['blocker'], before['process_exit']),
                                 ('blocked', 'console_assets', None), before)
                self.assertNotIn('command_started_at', before)
                stats.assert_not_called()
                history.append(blocked)
                snapshots = {key: (root / 'runs' / key / 'result.json').read_bytes() for key in history}
                generated = before['console_assets']['preparation'][-1]
                self.assertIn('--continue-checkout ' + blocked, generated)
                self.assertEqual(generated, before['recovery']['command'])
                self.assertNotIn('--reuse-checkout', generated)
                # Resource safety remains independent of the recoverable result class.
                recovery = dict(blocked_request, id=retry, continue_checkout=blocked)
                with runner.checkout_lease(root, owner), patch.object(runner, 'worker_alive', return_value=False):
                    for key in ('plan', 'origin', 'ref', 'executor_digest', 'schedule_digest', 'report_digest', 'console_digest'):
                        with self.subTest(changed=key), self.assertRaises(runner.RunBlocked):
                            runner.continue_checkout(root, dict(recovery, **{key: 'changed'}), owner)
                    for key in (owner, middle, latest):
                        with self.subTest(stale=key), self.assertRaises(runner.RunBlocked):
                            runner.continue_checkout(root, dict(recovery, continue_checkout=key), owner)
                    with self.assertRaises(runner.RunBlocked):
                        runner.verify_reused_checkout(root, dict(recovery, reuse_checkout=middle))
                    with patch.object(runner, 'worker_alive', return_value=True), self.assertRaises(runner.RunBlocked):
                        runner.continue_checkout(root, recovery, owner)
                    (checkout / 'dirty-source').touch()
                    with self.assertRaises(runner.RunBlocked):
                        runner.continue_checkout(root, recovery, owner)
                    (checkout / 'dirty-source').unlink()
                    state_path = root / ('checkout-' + owner + '.json')
                    saved_state = state_path.read_bytes()
                    runner.save(state_path, dict(run_id=middle, sha=common['sha']))
                    with self.assertRaises(runner.RunBlocked):
                        runner.continue_checkout(root, recovery, owner)
                    state_path.write_bytes(saved_state)
                    with self.assertRaises(subprocess.CalledProcessError):
                        runner.continue_checkout(root, dict(recovery, sha=first['sha']), owner)
                # Keep hashes/source checks active on repeated preparation failures.
                again, _ = self.execute_worker(root, recovery, self.dist)
                self.assertEqual(again.get('blocker'), 'console_assets', again)
                snapshots[retry] = (root / 'runs' / retry / 'result.json').read_bytes()
                (self.dist / assets.RECEIPT).write_text(json.dumps(assets.manifest(common['sha'], assets.files(self.dist))))
                argv = shlex.split(again['recovery']['command'])
                staging = Path(argv[argv.index('--frontend-dist') + 1])
                staging.parent.mkdir(parents=True, exist_ok=True)
                shutil.copytree(self.dist, staging)
                request = self.generated_request(again['recovery']['command'], recovered, common['executor_sha'])
                for key in ('sha', 'executor_sha', 'executor_digest', 'schedule_digest', 'report_digest',
                            'console_digest', 'timeout', 'plan', 'workbench', 'ssh_host'):
                    self.assertEqual(request[key], common[key], key)
                self.assertEqual((request['phase'], request['related_runs'], request['command']),
                                 ('final', [middle, latest], command))
                after, _ = self.execute_worker(root, request)
                self.assertEqual(after['status'], 'completed', after)
                self.assertEqual(after['process_exit'], 0)
                self.assertIn('command_started_at', after)
                self.assertEqual(after['tests']['counts']['passed'], 1)
                self.assertEqual((after['checkout'], after['checkout_owner']), (str(checkout), owner))
                self.assertEqual(after['console_assets']['source_revision'], common['sha'])
                self.assertEqual(sentinel.read_bytes(), b'retained compilation')
                for key, snapshot in snapshots.items():
                    self.assertEqual((root / 'runs' / key / 'result.json').read_bytes(), snapshot)
                metadata = dict(plan=common['plan'], related_runs=[blocked])
                with self.assertRaisesRegex(ValueError, 'has not executed'):
                    runner.reporting().related_reports(root / 'runs', metadata)

    def test_successful_owner_without_plan_reuses_checkout_after_a_failed_borrower(self):
        root = self.root / 'workbench'
        owner, failed, retry = ['20261024-03000' + str(i) + '-abcdef12' for i in range(1, 4)]
        arguments = ['python3', 'scripts/remote-rust.py', '--host', 'fixture-backend',
                     '--workbench', '.local/share/fixture-workbench', 'run',
                     '--ref', self.git('symbolic-ref', 'HEAD'), '--sha', self.sha,
                     '--retention', 'always']
        command = ['--', 'cargo', 'test', '--lib']
        initial = self.generated_request(shlex.join(arguments + command), owner)
        self.assertIsNone(initial.get('plan'))
        first, _ = self.execute_worker(root, initial)
        self.assertEqual((first['status'], first['process_exit']), ('completed', 0), first)
        checkout = Path(first['checkout'])
        (checkout / 'target').mkdir(exist_ok=True)
        sentinel = checkout / 'target/retained-compiler-objects'
        sentinel.write_bytes(b'retained compilation')
        reuse = shlex.join(arguments + ['--reuse-checkout', owner] + command)
        failure = self.root / 'home/fail-command'
        failure.touch()
        before, _ = self.execute_worker(root, self.generated_request(reuse, failed))
        failure.unlink()
        self.assertEqual((before['status'], before['process_exit']), ('failed', 101), before)
        self.assertFalse(before.get('timed_out'))
        self.assertIn('command_finished_at', before)
        snapshots = {key: (root / 'runs' / key / 'result.json').read_bytes() for key in (owner, failed)}
        after, _ = self.execute_worker(root, self.generated_request(reuse, retry))
        self.assertEqual((after['status'], after['process_exit']), ('completed', 0), after.get('error'))
        self.assertEqual(after['tests']['counts']['passed'], 1)
        self.assertEqual((after['checkout'], after['checkout_owner']), (str(checkout), owner))
        self.assertEqual((after['sha'], after['executor_digest']), (before['sha'], before['executor_digest']))
        self.assertEqual(sentinel.read_bytes(), b'retained compilation')
        self.assertEqual(json.loads((root / ('checkout-' + owner + '.json')).read_text())['run_id'], retry)
        for key, snapshot in snapshots.items():
            self.assertEqual((root / 'runs' / key / 'result.json').read_bytes(), snapshot)

    def test_on_failure_setup_owner_reuses_checkout_after_generated_recovery_test_fails(self):
        root = self.root / 'workbench'
        owner, failed, retry = ['20261024-04000' + str(i) + '-abcdef12' for i in range(1, 4)]
        initial = self.generated_request(shlex.join([
            'python3', 'scripts/remote-rust.py', '--host', 'fixture-backend',
            '--workbench', '.local/share/fixture-workbench', 'run', '--ref', self.git('symbolic-ref', 'HEAD'),
            '--sha', self.sha, '--retention', 'on-failure', '--plan', 'console-retry', '--',
            'cargo', 'test', '--test', 'publication_process', 'installed_']), owner)
        self.assertEqual(initial['retention'], 'on-failure')
        first, stats = self.execute_worker(root, initial)
        self.assertEqual((first['status'], first['blocker'], first['process_exit']),
                         ('blocked', 'console_assets', None), first)
        stats.assert_not_called()
        checkout = Path(first['checkout'])
        (checkout / 'target').mkdir(exist_ok=True)
        sentinel = checkout / 'target/retained-compiler-objects'
        sentinel.write_bytes(b'retained compilation')
        recovery = first['recovery']['command']
        argv = shlex.split(recovery)
        self.assertEqual(argv[argv.index('--reuse-checkout') + 1], owner)
        staging = Path(argv[argv.index('--frontend-dist') + 1])
        staging.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(self.dist, staging)
        borrower = self.generated_request(recovery, failed)
        self.assertEqual(borrower['retention'], 'always')
        failure = self.root / 'home/fail-command'
        failure.touch()
        before, _ = self.execute_worker(root, borrower)
        failure.unlink()
        self.assertEqual((before['status'], before['process_exit']), ('failed', 101), before)
        self.assertFalse(before.get('timed_out'))
        self.assertIn('command_finished_at', before)
        self.assertEqual(before['console_assets']['state'], 'verified')
        snapshots = {key: (root / 'runs' / key / 'result.json').read_bytes() for key in (owner, failed)}
        after, _ = self.execute_worker(root, self.generated_request(recovery, retry))
        self.assertEqual((after['status'], after['process_exit']), ('completed', 0), after.get('error'))
        self.assertEqual(after['tests']['counts']['passed'], 1)
        self.assertEqual((after['checkout'], after['checkout_owner']), (str(checkout), owner))
        self.assertEqual((after['sha'], after['executor_digest']), (before['sha'], before['executor_digest']))
        self.assertEqual(after['console_assets']['source_revision'], self.sha)
        self.assertEqual(sentinel.read_bytes(), b'retained compilation')
        self.assertEqual(json.loads((root / ('checkout-' + owner + '.json')).read_text())['run_id'], retry)
        for key, snapshot in snapshots.items():
            self.assertEqual((root / 'runs' / key / 'result.json').read_bytes(), snapshot)

    def test_cache_preflight_uses_the_same_generated_recovery_and_rechecks_setup(self):
        root = self.root / 'workbench'
        owner, retry = '20261024-020001-abcdef12', '20261024-020002-abcdef12'
        initial = self.generated_request(shlex.join([
            'python3', 'scripts/remote-rust.py', '--host', 'fixture-backend',
            '--workbench', '.local/share/fixture-workbench', 'run', '--ref', self.git('symbolic-ref', 'HEAD'),
            '--sha', self.sha, '--retention', 'always', '--plan', 'cache-recovery', '--', 'cargo', 'test', '--lib']), owner)
        blocked, stats = self.execute_worker(root, initial,
            cache_error=runner.RunBlocked('cache_server_unhealthy', 'fixture cache admission failure'))
        self.assertEqual((blocked['status'], blocked['blocker'], blocked['process_exit']),
                         ('blocked', 'cache_server_unhealthy', None))
        self.assertNotIn('command_started_at', blocked)
        stats.assert_not_called()
        self.assertIn('--continue-checkout ' + owner, blocked['recovery']['command'])
        old_result = (root / 'runs' / owner / 'result.json').read_bytes()
        after, _ = self.execute_worker(root, self.generated_request(blocked['recovery']['command'], retry))
        self.assertEqual(after['status'], 'completed', after)
        self.assertEqual(after['checkout'], blocked['checkout'])
        self.assertEqual((root / 'runs' / owner / 'result.json').read_bytes(), old_result)


if __name__ == '__main__':
    unittest.main()
