#!/usr/bin/env python3
"""Cleanup safety tests: real temporary Git worktrees, no Rust compilation."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("local_rust", Path(__file__).with_name("local-rust.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class Cleanup(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.repo = self.base / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        m.git(self.repo, "config", "core.hooksPath", "/dev/null")
        m.git(self.repo, "config", "user.name", "Fixture")
        m.git(self.repo, "config", "user.email", "fixture@example.invalid")
        (self.repo / "source").write_text("preserve source")
        (self.repo / ".gitignore").write_text("target/\n")
        m.git(self.repo, "add", ".")
        m.git(self.repo, "commit", "-qm", "fixture")
        self.store = m.Store(self.base / "state", self.base / "global.lock")

    def target(self, checkout):
        debug = checkout / "target/debug"
        debug.mkdir(parents=True)
        (debug / "binary").write_bytes(b"rebuildable")
        evidence = checkout / "target/e2e"
        evidence.mkdir()
        (evidence / "result.json").write_text('{"scenario":"red"}')

    def developer(self):
        self.target(self.repo)
        return self.store.retire(self.repo)

    def managed(self):
        key = m.uuid.uuid4().hex
        checkout = self.store.root / "checkouts" / key
        m.git(self.repo, "worktree", "add", "--detach", str(checkout), "HEAD")
        self.target(checkout)
        row = dict(id=key, kind="managed", repo=str(self.repo), checkout=str(checkout),
                   identity=m.identity(checkout), sha=m.git(checkout, "rev-parse", "HEAD"),
                   status="terminal", scenario="red", process_exit=1, keep=False,
                   evidence_saved=True, finished_at=0,
                   target_identity=m.identity(checkout / "target"),
                   debug_identity=m.identity(checkout / "target/debug"))
        self.store.save(row)
        return row

    def test_preview_is_non_destructive(self):
        row = self.developer()
        before = self.store.record_path(row['id']).read_bytes()
        self.assertEqual(self.store.preview()[0]['state'], 'candidate')
        self.assertEqual(before, self.store.record_path(row['id']).read_bytes())
        self.assertTrue((self.repo / 'target/debug/binary').exists())

    def targetless_admission(self):
        row = self.managed()
        m.shutil.rmtree(Path(row['checkout']) / 'target')
        row.pop('target_identity')
        row.pop('debug_identity')
        row['process_exit'] = None
        self.store.save(row)
        return row

    def test_targetless_admission_cleanup_preserves_run_evidence(self):
        row = self.targetless_admission()
        report = self.store.record_path(row['id']).parent / 'validation-report.json'
        report.write_text('{"scenario":"not_executed"}')
        candidate = self.store.preview_one(row)
        self.assertEqual(candidate['state'], 'candidate')
        self.assertEqual(candidate['bytes'], 0)
        self.store.apply(row['id'], candidate['token'])
        self.assertFalse(Path(row['checkout']).exists())
        self.assertTrue(self.store.load(row['id'])['removed'])
        self.assertIsNone(self.store.load(row['id'])['process_exit'])
        self.assertEqual(json.loads(report.read_text())['scenario'], 'not_executed')

    def test_targetless_admission_rejects_later_target_and_dangling_link(self):
        row = self.targetless_admission()
        candidate = self.store.preview_one(row)
        target = Path(row['checkout']) / 'target'
        for linked in (False, True):
            if linked:
                target.symlink_to(self.base / 'missing', target_is_directory=True)
            else:
                target.mkdir()
            with self.assertRaises((ValueError, KeyError)):
                self.store.apply(row['id'], candidate['token'])
            if linked:
                target.unlink()
            else:
                target.rmdir()
        self.assertTrue(Path(row['checkout']).exists())

    def test_missing_target_after_started_or_attested_run_stays_protected(self):
        row = self.targetless_admission()
        for extra in ({'command_started_at': 1}, {'process_exit': 1},
                      {'target_identity': [1, 2]}, {'debug_identity': [1, 3]}):
            self.assertEqual(self.store.preview_one(dict(row, **extra))['state'], 'skipped')

    def test_developer_cleanup_preserves_dirty_source_evidence_and_branch(self):
        row = self.developer()
        (self.repo / 'source').write_text('uncommitted')
        (self.repo / 'new-file').write_text('untracked')
        candidate = self.store.preview_one(row)
        self.store.apply(row['id'], candidate['token'])
        self.assertFalse((self.repo / 'target/debug').exists())
        self.assertEqual((self.repo / 'source').read_text(), 'uncommitted')
        self.assertTrue((self.repo / 'new-file').exists())
        self.assertTrue((self.repo / 'target/e2e/result.json').exists())
        self.assertEqual(m.git(self.repo, 'rev-parse', 'HEAD'), row['sha'])

    def test_activity_lock_blocks_preview_and_apply(self):
        row = self.developer()
        token = self.store.preview_one(row)['token']
        with self.store.locked(self.repo):
            self.assertEqual(self.store.preview_one(row)['state'], 'skipped')
            with self.assertRaises(BlockingIOError):
                self.store.apply(row['id'], token)

    def test_child_holding_global_lock_blocks_cleanup(self):
        row = self.developer()
        code = 'import fcntl,sys; f=open(sys.argv[1],"a"); fcntl.flock(f,fcntl.LOCK_EX); print("ready",flush=True); sys.stdin.read()'
        with subprocess.Popen([m.sys.executable, '-c', code, str(self.store.global_lock)],
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as child:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            self.assertEqual(self.store.preview_one(row)['state'], 'skipped')
            child.communicate('done')
            self.assertEqual(child.returncode, 0)

    def test_keep_and_missing_evidence_are_protected(self):
        row = self.managed()
        for field, value in [('keep', True), ('evidence_saved', False), ('status', 'running')]:
            changed = dict(row, **{field: value})
            self.assertEqual(self.store.preview_one(changed)['state'], 'skipped')

    def test_snapshot_changes_reject_apply(self):
        row = self.developer()
        token = self.store.preview_one(row)['token']
        (self.repo / 'target/debug/binary').write_text('changed contents after preview')
        with self.assertRaisesRegex(ValueError, 'Candidate changed'):
            self.store.apply(row['id'], token)

    def test_replaced_target_or_symlink_is_rejected(self):
        row = self.developer()
        token = self.store.preview_one(row)['token']
        original = self.repo / 'target'
        original.rename(self.repo / 'saved')
        original.symlink_to(self.repo / 'saved', target_is_directory=True)
        with self.assertRaises(ValueError):
            self.store.apply(row['id'], token)
        self.assertTrue((self.repo / 'saved/debug/binary').exists())

    def test_debug_inner_symlink_never_deletes_external_target(self):
        row = self.developer()
        outside = self.base / 'outside'
        outside.mkdir()
        (outside / 'precious').write_text('keep')
        (self.repo / 'target/debug/link').symlink_to(outside, target_is_directory=True)
        self.store.apply(row['id'], self.store.preview_one(row)['token'])
        self.assertTrue((outside / 'precious').exists())

    def test_git_tracked_target_is_rejected(self):
        self.target(self.repo)
        m.git(self.repo, 'add', '-f', 'target/debug/binary')
        with self.assertRaisesRegex(ValueError, 'tracked'):
            self.store.retire(self.repo)

    def test_gc_ages_only_managed_and_keep_protects(self):
        dev = self.developer()
        row = self.managed()
        row['finished_at'] = time.time()
        self.store.save(row)
        self.assertEqual(self.store.gc(), [])
        row.update(finished_at=0, keep=True)
        self.store.save(row)
        self.assertEqual(self.store.gc(apply=True)[0]['state'], 'skipped')
        self.assertTrue((self.repo / 'target/debug').exists())
        row['keep'] = False
        self.store.save(row)
        self.assertEqual(self.store.gc(apply=True)[0]['state'], 'removed')
        self.assertTrue((self.repo / 'target/debug').exists())
        evidence = self.store.record_path(row['id']).parent / 'artifacts/e2e/result.json'
        self.assertEqual(json.loads(evidence.read_text())['scenario'], 'red')
        self.assertFalse(Path(row['checkout']).exists())
        self.assertFalse(self.store.load(dev['id']).get('removed', False))

    def test_managed_source_changes_block_cleanup(self):
        row = self.managed()
        (Path(row['checkout']) / 'source').write_text('do not lose')
        self.assertEqual(self.store.preview_one(row)['state'], 'skipped')

    def test_artifact_links_retain_checkout(self):
        row = self.managed()
        (Path(row['checkout']) / 'target/e2e/link').symlink_to(self.repo / 'source')
        with self.assertRaisesRegex(ValueError, 'Artifact contains links'):
            self.store.apply(row['id'], self.store.preview_one(row)['token'])
        self.assertTrue(Path(row['checkout']).exists())

    def test_pressure_never_removes_developer_target(self):
        self.developer()
        low = m.shutil._ntuple_diskusage(100 * m.GIB, 99 * m.GIB, m.GIB)
        with patch.object(m.shutil, 'disk_usage', return_value=low):
            with self.assertRaisesRegex(ValueError, 'build not started'):
                self.store.ensure_space()
        self.assertTrue((self.repo / 'target/debug/binary').exists())

    def test_unloaded_toolchain_refuses_before_record_worktree_or_space_check(self):
        empty = self.base / 'empty-bin'
        empty.mkdir()
        args = types.SimpleNamespace(repo=str(self.repo), ref=m.git(self.repo, 'symbolic-ref', 'HEAD'),
                                     sha=m.git(self.repo, 'rev-parse', 'HEAD'),
                                     command=['cargo', 'test'], keep=False, cargo_only=True, timeout=10)
        with patch.dict(os.environ, {'PATH': str(empty)}), \
             patch.object(m, 'TOOLCHAIN_BIN', empty), \
             patch.object(self.store, 'ensure_space') as space:
            with self.assertRaisesRegex(ValueError, 'Missing on PATH: cargo, rustc, sccache, git'):
                m.run(self.store, args)
            space.assert_not_called()
        self.assertEqual(list((self.store.root / 'runs').iterdir()), [])
        self.assertEqual(list((self.store.root / 'checkouts').iterdir()), [])

    def test_toolchain_lookup_appends_rustup_bin_after_loaded_path(self):
        loaded = self.base / 'loaded-bin'
        loaded.mkdir()
        fallback = self.base / 'rustup-bin'
        fallback.mkdir()
        for name in ('cargo', 'rustc', 'sccache', 'git'):
            tool = fallback / name
            tool.write_text('#!/bin/sh\n')
            tool.chmod(0o700)
        with patch.dict(os.environ, {'PATH': str(loaded)}), \
             patch.object(m, 'TOOLCHAIN_BIN', fallback):
            self.assertEqual(m.toolchain_environment(),
                             os.pathsep.join([str(loaded), str(fallback)]))

    def run_fixture(self, exit_code=0, cargo_only=True, delay=0, source="origin", jobs=2, command=None):
        # Real git origin/worktree and subprocess lifecycle; fake compiler means NO Rust proof.
        m.git(self.repo, 'remote', 'add', 'origin', str(self.base / 'unpublished') if source == 'local' else str(self.repo))
        branch = m.git(self.repo, 'symbolic-ref', 'HEAD')
        binary = self.base / 'bin'
        binary.mkdir()
        cargo = binary / 'cargo'
        cargo.write_text('#!' + m.sys.executable + '\n'
                         'import sys,json,pathlib\n'
                         'p=pathlib.Path.cwd()/"target"\n'
                         'if sys.argv[1]=="metadata":\n'
                         ' print(json.dumps({"target_directory":str(p)}))\n'
                         'elif sys.argv[1]=="--version":\n'
                         ' print("cargo fixture")\n'
                         'else:\n'
                         ' (p/"debug").mkdir(parents=True)\n'
                         ' (p/"debug/binary").write_text("fixture")\n'
                         ' import time; time.sleep(' + str(delay) + ')\n'
                         ' print("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out")\n'
                         ' sys.exit(' + str(exit_code) + ')\n')
        cargo.chmod(0o700)
        for name in ('rustc', 'sccache'):
            tool = binary / name
            tool.write_text('#!/bin/sh\nexit 0\n')
            tool.chmod(0o700)
        args = types.SimpleNamespace(repo=str(self.repo), ref=branch,
                                     sha=m.git(self.repo, 'rev-parse', 'HEAD'),
                                     command=command or ['cargo', 'test'], keep=False,
                                     cargo_only=cargo_only, timeout=1 if delay else 10,
                                     source=source, jobs=jobs)
        with patch.dict(os.environ, {'PATH': str(binary) + os.pathsep + os.environ['PATH']}), \
             patch.object(m.remote, 'stats', return_value={'cache_hits': 1}), \
             patch.object(self.store, 'ensure_space'):
            result = m.run(self.store, args)
        if result.get('temporary_directory'):
            self.addCleanup(m.remote.cleanup_temp_directory, result['temporary_directory'])
        return result

    def test_workspace_default_failure_policy_is_preserved(self):
        command = ['cargo', 'test', '--workspace', '--exclude', 'hiroute-desktop']
        result = self.run_fixture(command=command)
        self.assertEqual(result['process_exit'], 0)
        self.assertEqual(result['command'], command)

    def test_explicit_collect_all_failure_policy_is_preserved(self):
        command = ['cargo', 'test', '--no-fail-fast', '--workspace', '--exclude', 'hiroute-desktop']
        result = self.run_fixture(command=command)
        self.assertEqual(result['process_exit'], 0)
        self.assertEqual(result['command'], command)

    def test_local_candidate_needs_no_published_origin_and_preserves_dirty_tree(self):
        (self.repo / 'source').write_text('uncommitted source must stay outside build')
        result = self.run_fixture(source='local', cargo_only=False, jobs=3)
        self.assertEqual(result['process_exit'], 0)
        self.assertEqual(result['scenario'], 'unassessed')
        self.assertEqual(result['candidate_source'], 'local')
        self.assertEqual(result['build_jobs'], 3)
        self.assertEqual((Path(result['checkout']) / 'source').read_text(), 'preserve source')
        self.assertEqual((self.repo / 'source').read_text(), 'uncommitted source must stay outside build')

    def test_managed_cargo_run_keeps_feedback_after_automatic_cleanup(self):
        result = self.run_fixture(source='local')
        self.assertEqual(result['process_exit'], 0)
        self.assertFalse(Path(result['checkout']).exists())
        report = json.loads(Path(result['validation_report']['path']).read_text())
        self.assertEqual(report['sha'], result['sha'])
        self.assertEqual(report['process_exit'], 0)
        # This fake tool prints only an unattributed summary; it is not target evidence.
        self.assertFalse(report['complete'])
        self.assertIsNotNone(report['timing']['command_seconds'])

    def test_invalid_jobs_refuses_before_run_record(self):
        with self.assertRaisesRegex(ValueError, 'positive build jobs'):
            self.run_fixture(jobs=0)
        self.assertEqual(list((self.store.root / 'runs').iterdir()), [])

    def test_managed_run_success_exports_log_and_removes_checkout(self):
        result = self.run_fixture()
        self.assertEqual(result['process_exit'], 0)
        self.assertEqual(result['scenario'], 'green')
        self.assertTrue(result['removed'])
        self.assertTrue((self.store.record_path(result['id']).parent / 'command.log').exists())
        self.assertFalse(Path(result['checkout']).exists())

    def test_managed_run_failure_retains_checkout_and_temp(self):
        result = self.run_fixture(exit_code=4)
        self.assertEqual(result['process_exit'], 4)
        self.assertEqual(result['scenario'], 'red')
        self.assertTrue(Path(result['checkout']).exists())
        self.assertTrue(Path(result['temporary_directory']['path']).exists())

    def test_zero_exit_without_scenario_assessment_retains(self):
        result = self.run_fixture(cargo_only=False)
        self.assertEqual(result['process_exit'], 0)
        self.assertEqual(result['scenario'], 'unassessed')
        self.assertFalse(result['evidence_saved'])
        self.assertTrue(Path(result['checkout']).exists())

    def test_timeout_reaps_child_and_releases_locks_without_cleanup(self):
        result = self.run_fixture(delay=30)
        self.assertEqual(result['process_exit'], -9)
        self.assertEqual(result['scenario'], 'red')
        self.assertTrue(Path(result['checkout']).exists())
        with self.store.locked(result['checkout']):
            pass

    def test_short_canonical_temporary_directory(self):
        info = m.private_temp()
        self.addCleanup(m.remote.cleanup_temp_directory, info)
        p = Path(info['path'])
        self.assertEqual(p.resolve(), p)
        self.assertEqual(p.stat().st_mode & 0o777, 0o700)
        self.assertLessEqual(len(os.fsencode(p)), m.remote.MAX_TMPDIR_BYTES)

    @unittest.skipUnless(os.environ.get('HIROUTE_REAL_CARGO_TEST') == '1',
                         'Opt-in tiny platform-runner integration, not HiRoute product tests')
    def test_real_cargo_pushed_fixture_lifecycle(self):
        (self.repo / 'Cargo.toml').write_text(
            '[package]\nname="retention-fixture"\nversion="0.1.0"\nedition="2021"\n')
        (self.repo / 'src').mkdir()
        (self.repo / 'src/lib.rs').write_text(
            '#[test] fn proof() { assert_eq!(2 + 2, 4); }\n')
        subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=self.repo, check=True)
        m.git(self.repo, 'add', 'Cargo.toml', 'Cargo.lock', 'src')
        m.git(self.repo, 'commit', '-qm', 'tiny local runner fixture')
        origin = self.base / 'origin.git'
        subprocess.run(['git', 'init', '--bare', '-q', str(origin)], check=True)
        m.git(self.repo, 'remote', 'add', 'origin', str(origin))
        branch = m.git(self.repo, 'symbolic-ref', 'HEAD')
        m.git(self.repo, 'push', 'origin', branch)
        self.store.global_lock = m.HOST_LOCK
        args = types.SimpleNamespace(repo=str(self.repo), ref=branch,
                                     sha=m.git(self.repo, 'rev-parse', 'HEAD'),
                                     command=['cargo', 'test', '--locked', '--offline'],
                                     keep=False, cargo_only=True, timeout=120)
        result = m.run(self.store, args)
        self.assertEqual(result['process_exit'], 0, result)
        self.assertEqual(result['scenario'], 'green', result)
        self.assertTrue(result['removed'], result)
        self.assertFalse(Path(result['checkout']).exists())
        def report(row):
            values = {k: row[k] for k in ('sha', 'command', 'process_exit', 'tests', 'scenario')}
            values['cache'] = {phase: {
                'hits': row[phase]['stats']['cache_hits']['counts'],
                'misses': row[phase]['stats']['cache_misses']['counts'],
                'max_bytes': row[phase]['max_cache_size'],
            } for phase in ('cache_before', 'cache_after')}
            print(json.dumps(values))
        report(result)
        (self.repo / 'src/lib.rs').write_text('#[test] fn intended_failure() { panic!("fixture"); }\n')
        m.git(self.repo, 'add', 'src/lib.rs')
        m.git(self.repo, 'commit', '-qm', 'intentional diagnostic retention fixture')
        m.git(self.repo, 'push', 'origin', branch)
        args.sha = m.git(self.repo, 'rev-parse', 'HEAD')
        failed = m.run(self.store, args)
        self.addCleanup(m.remote.cleanup_temp_directory, failed['temporary_directory'])
        self.assertEqual(failed['process_exit'], 101)
        self.assertEqual(failed['scenario'], 'red')
        self.assertTrue(Path(failed['checkout']).exists())
        self.assertTrue(Path(failed['temporary_directory']['path']).exists())
        report(failed)


if __name__ == '__main__':
    unittest.main()
