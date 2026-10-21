#!/usr/bin/env python3
"""Console selection, provenance and real managed-checkout admission regressions."""
import base64
import copy
import importlib.util
import json
import os
from pathlib import Path
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
                    'linux', {'invalid': 'unused'}), dict(state='not_required', cases=[]))
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

    def test_transfer_rejects_old_sha_wrong_hash_missing_extra_and_linked_files(self):
        value = assets.bundle(self.dist)
        destination = self.root / 'destination'
        (destination / 'apps/desktop').mkdir(parents=True)
        assets.stage(destination, self.sha, value)
        self.assertEqual(assets.verify(destination / 'apps/desktop/dist', self.sha), value['receipt'])
        for kind in ('old', 'hash', 'missing', 'extra', 'traversal'):
            bad = copy.deepcopy(value)
            if kind == 'old':
                bad['receipt']['source_revision'] = '0' * 40
            elif kind == 'hash':
                bad['contents']['index.html'] = base64.b64encode(b'corrupted').decode()
            elif kind == 'missing':
                del bad['contents']['index.html']
            else:
                bad['contents']['../foreign' if kind == 'traversal' else 'extra'] = ''
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                assets.stage(destination, self.sha, bad)
        (self.dist / 'assets/app.js').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'hashes'):
            assets.bundle(self.dist)
        (self.dist / 'assets/app.js').unlink()
        (self.dist / 'assets/app.js').symlink_to(self.dist / 'index.html')
        with self.assertRaisesRegex(ValueError, 'link'):
            assets.bundle(self.dist)

    def test_clean_worker_blocks_before_rust_then_reuses_same_checkout_with_verified_assets(self):
        root = self.root / 'workbench'
        first = '20261021-000001-abcdef12'
        second = '20261021-000002-abcdef12'
        ref = self.git('symbolic-ref', 'HEAD')
        request = dict(id=first, ref=ref, sha=self.sha, executor_sha=self.sha, origin=str(self.repo),
                       command=['cargo', 'test', '-p', 'hiroute-daemon', '--test', 'publication_process',
                                'installed_', '--', '--nocapture'],
                       retention='on-failure', exclusive=False, timeout=10, plan='console-assets', phase='focused')
        home = self.root / 'home'
        binary = home / '.cargo/bin'
        binary.mkdir(parents=True)
        fake = '''import sys,json,pathlib
if '--version' in sys.argv:
 print(pathlib.Path(sys.argv[0]).name+' fixture');sys.exit(0)
if 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(pathlib.Path.cwd()/'target')}));sys.exit(0)
print('Running tests/publication_process.rs (target/debug/deps/publication_process-abcdef0123456789)')
print('running 1 test')
print('test component_fixture ... ok')
print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
'''
        for name in ('cargo', 'rustc'):
            path = binary / name
            path.write_text('#!' + sys.executable + '\n' + fake)
            path.chmod(0o700)

        def execute(row, bundle=None):
            run = root / 'runs' / row['id']
            run.mkdir(parents=True)
            runner.save(run / 'request.json', row)
            if bundle is not None:
                runner.save(run / 'console-assets.json', bundle)
            with patch.object(runner.Path, 'home', return_value=home), patch.dict(os.environ, {}, clear=False), \
                 patch.object(runner.sys, 'platform', 'linux'), \
                 patch.object(runner.tempfile, 'gettempdir', return_value=str(self.root)), \
                 patch.object(runner, 'MIN_FREE_BYTES', 0), \
                 patch.object(runner, 'stats', return_value={'max_cache_size': 30 * 1024**3}) as stats:
                runner.worker(run / 'request.json')
            return json.loads((run / 'result.json').read_text()), stats

        before, stats = execute(request)
        self.assertEqual((before['status'], before['blocker'], before['process_exit']),
                         ('blocked', 'console_assets', None), before)
        self.assertNotIn('command_started_at', before)
        stats.assert_not_called()
        self.assertFalse((root / 'runs' / first / 'command.log').exists())
        self.assertIn('--reuse-checkout ' + first, before['console_assets']['preparation'][-1])
        row = dict(request, id=second, reuse_checkout=first, retention='always')
        lease = runner.checkout_lease(root, first)
        try:
            blocked, _ = execute(dict(row, id='20261021-000003-abcdef12'), assets.bundle(self.dist))
            self.assertEqual(blocked.get('blocker'), 'checkout_busy')
        finally:
            lease.close()
        for index, kind in enumerate(('sha', 'hash'), 4):
            bad = assets.bundle(self.dist)
            if kind == 'sha':
                bad['receipt']['source_revision'] = '0' * 40
            else:
                bad['contents']['index.html'] = base64.b64encode(b'corrupted').decode()
            invalid, stats = execute(dict(row, id='20261021-00000' + str(index) + '-abcdef12'), bad)
            self.assertEqual(invalid.get('blocker'), 'console_assets')
            self.assertNotIn('command_started_at', invalid)
            stats.assert_not_called()
        after, _ = execute(row, assets.bundle(self.dist))
        self.assertEqual(after['status'], 'completed', after)
        self.assertEqual(after['checkout'], before['checkout'])
        self.assertEqual(after['console_assets']['state'], 'verified')
        (Path(after['checkout']) / 'apps/desktop/dist/assets/app.js').write_text('damaged retained asset')
        invalid, stats = execute(dict(row, id='20261021-000006-abcdef12'))
        self.assertEqual(invalid.get('blocker'), 'console_assets')
        stats.assert_not_called()
        self.assertEqual(json.loads((root / 'runs' / first / 'result.json').read_text())['status'], 'blocked')
        runner.cleanup_temp_directory(after['temporary_directory'])


if __name__ == '__main__':
    unittest.main()
