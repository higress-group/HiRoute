#!/usr/bin/env python3
"""Fixture-only local checks; --real-consumer selects the required bounded Rust CI case."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("builds", Path(__file__).with_name("pilot-builds.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
REAL_RUSTC = shutil.which('rustc')
REAL_PATH = os.environ.get('PATH', '')
CONSUMER_EVIDENCE = Path(__file__).resolve().parents[1] / 'artifacts/ci/pilot-manifest-consumer'


def consumer_step(phase, argv, cwd, env, timeout):
    """Keep each real-consumer subprocess result outside the disposable fixture."""
    report = {'phase': phase, 'returncode': None}
    try:
        completed = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=timeout)
        report.update(returncode=completed.returncode, stdout=completed.stdout, stderr=completed.stderr)
    except (OSError, subprocess.TimeoutExpired) as error:
        report.update(error=str(error), stdout=getattr(error, 'stdout', None), stderr=getattr(error, 'stderr', None))
    for key in ('stdout', 'stderr'):
        value = report.get(key) or b''
        report[key] = value.decode('utf-8', errors='replace') if isinstance(value, bytes) else value
    CONSUMER_EVIDENCE.mkdir(parents=True, exist_ok=True)
    evidence = CONSUMER_EVIDENCE / (phase + '.json')
    evidence.write_text(json.dumps(report, indent=2) + '\n')
    if report['returncode'] != 0:
        message = 'Real consumer phase %s failed; diagnostics: %s' % (phase, evidence)
        print(message, file=sys.stderr)
        raise RuntimeError(message)
    return completed.stdout


class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.repo)], check=True)
        for key, value in [('user.name', 'Fixture'), ('user.email', 'fixture@example.invalid'), ('core.hooksPath', '/dev/null')]:
            m.local.git(self.repo, 'config', key, value)
        (self.repo / '.gitignore').write_text('target/\napps/desktop/node_modules/\napps/desktop/dist/\napps/desktop/src-tauri/gen/\n')
        (self.repo / 'rust-toolchain.toml').write_text('[toolchain]\nchannel = "fixture"\n')
        (self.repo / 'scripts').mkdir()
        (self.repo / 'scripts/desktop-pilot.py').write_text('import json,sys\nprint(json.dumps({"build":{"frontendDist":sys.argv[-1]}}))\n')
        native = self.repo / 'apps/desktop/src-tauri'
        native.mkdir(parents=True)
        (native / 'development-cpa-artifacts.v1.json').write_text('{"fixture":"default"}\n')
        m.local.git(self.repo, 'add', '.')
        m.local.git(self.repo, 'commit', '-qm', 'fixture')
        m.local.git(self.repo, 'remote', 'add', 'origin', str(self.repo))
        self.branch = m.local.git(self.repo, 'symbolic-ref', 'HEAD')
        self.sha = m.local.git(self.repo, 'rev-parse', 'HEAD')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        tools = '''import json,os,pathlib,sys
name=pathlib.Path(sys.argv[0]).name
if '--version' in sys.argv or '-vV' in sys.argv:
 print(name+' fixture-1');sys.exit(0)
root=pathlib.Path.cwd()
if name=='cargo' and 'metadata' in sys.argv:
 print(json.dumps({'target_directory':str(root/'target')}));sys.exit(0)
if name=='cargo':
 p=root/'target/debug';p.mkdir(parents=True)
 manifest=os.environ.get('HIROUTE_CPA_MANIFEST')
 manifest=pathlib.Path(manifest) if manifest is not None else root/'apps/desktop/src-tauri/development-cpa-artifacts.v1.json'
 if not manifest.is_absolute(): manifest=root/'apps/desktop/src-tauri'/manifest
 (p/'compiled-cpa.json').write_bytes(manifest.read_bytes())
 for name in ('hiroute-desktop','hirouted','hiroute'):
  (p/name).write_text('#!/bin/sh\\nexit 0\\n');(p/name).chmod(0o700)
 gen=root/'apps/desktop/src-tauri/gen';gen.mkdir(parents=True);(gen/'schema').write_text('generated')
 (root/'target/evidence').mkdir();(root/'target/evidence/result').write_text('unique evidence')
elif name=='npm':
 base=root/'apps/desktop';base.mkdir(parents=True,exist_ok=True)
 p=base/('node_modules' if 'ci' in sys.argv else 'dist');p.mkdir(exist_ok=True)
 (p/('dependency' if 'ci' in sys.argv else 'index.html')).write_text('fixture')
print('fixture build completed')
'''
        for name in ('cargo', 'rustc', 'sccache', 'npm', 'node', 'cc', 'cmake'):
            path = self.bin / name
            path.write_text('#!' + sys.executable + '\n' + tools)
            path.chmod(0o700)
        env = patch.dict(os.environ, {'PATH': str(self.bin) + os.pathsep + os.environ['PATH']})
        env.start()
        self.addCleanup(env.stop)
        os.environ.pop('HIROUTE_CPA_MANIFEST', None)
        stats = patch.object(m.local.remote, 'stats', return_value={'cache_hits': 0})
        stats.start()
        self.addCleanup(stats.stop)
        admission = patch.object(m.local.remote, 'admit_cache_server', return_value={'state': 'ready'})
        admission.start()
        self.addCleanup(admission.stop)
        self.store = m.local.Store(self.root / 'state', self.root / 'host.lock')
        self.manager = m.Builds(self.store)

    def acquire(self, owner='task-one', cases=('model',), jobs=2):
        return self.manager.acquire(types.SimpleNamespace(repo=str(self.repo), ref=self.branch, sha=self.sha,
                                                         owner=owner, case=list(cases), jobs=jobs, timeout=20))

    def start(self, result, case='model'):
        def launch(args):
            root = Path(args.root)
            root.mkdir(mode=0o700)
            self.addCleanup(shutil.rmtree, root)
            data = root / 'data'
            data.mkdir(mode=0o700)
            session = dict(root=str(root), owner_uid=os.getuid(), build_run=args.build_run,
                           source_sha=args.source_sha, data_root=str(data))
            m.pilot.write_private_json(root / 'session.json', session)
            print(json.dumps(dict(session=str(root / 'session.json'), socket=str(root / 'pilot.sock'))))
            return 0
        with patch.object(m.pilot, 'start', side_effect=launch):
            return self.manager.start(types.SimpleNamespace(lease=result['lease'], case=case, timeout=1,
                                                           data_root=None, process_home=None))

    def finish(self, result, verdicts=('model=not_executed',), stop_error=None, debug=True):
        diagnostics = {'roles': {role: {'evidence': {'level_applied': {'level': 'debug' if debug else 'info'}}}
                                 for role in ('desktop', 'daemon')}}
        with patch.object(m.pilot, 'stop_group', side_effect=stop_error, return_value=[]), \
                patch.object(m.pilot, 'diagnostics_index', return_value=diagnostics):
            return self.manager.finish(types.SimpleNamespace(lease=result['lease'], case=list(verdicts), evidence_saved=True))

    def test_same_candidate_builds_once_and_same_owner_is_idempotent(self):
        first = self.acquire()
        second = self.acquire()
        third = self.acquire('task-two')
        self.assertFalse(first['reused'])
        self.assertTrue(second['reused'])
        self.assertEqual(first['lease'], second['lease'])
        self.assertNotEqual(first['lease'], third['lease'])
        self.assertEqual(first['build_run'], third['build_run'])
        self.assertEqual(len(list((self.store.root / 'checkouts').iterdir())), 1)

    def test_frontend_is_same_candidate_and_generated_dependencies_are_discarded(self):
        result = self.acquire()
        row = self.store.load(result['build_run'])
        self.assertEqual(m.local.git(Path(row['checkout']), 'rev-parse', 'HEAD'), self.sha)
        for directory in ('node_modules', 'dist', 'src-tauri/gen'):
            self.assertFalse((Path(row['checkout']) / 'apps/desktop' / directory).exists())
        self.assertEqual(m.tree_digest(Path(row['pilot_frontend']['path'])), row['pilot_frontend']['sha256'])

    def test_changed_build_jobs_cannot_share_bundle(self):
        first = self.acquire(jobs=2)
        second = self.acquire(jobs=3)
        self.assertNotEqual(first['build_run'], second['build_run'])

    def test_changed_build_environment_cannot_share_bundle(self):
        first = self.acquire()
        with patch.dict(os.environ, {'RUSTFLAGS': '-C debuginfo=1'}):
            second = self.acquire('task-two')
        self.assertNotEqual(first['build_run'], second['build_run'])

    def test_equal_cpa_manifest_contents_share_a_build_across_paths(self):
        first_manifest = self.root / 'private-first-cpa.json'
        same_content_manifest = self.root / 'private-same-cpa.json'
        second_manifest = self.root / 'private-second-cpa.json'
        first_manifest.write_text('{"fixture":"first"}\n')
        same_content_manifest.write_text('{"fixture":"first"}\n')
        second_manifest.write_text('{"fixture":"second"}\n')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(first_manifest)}):
            first = self.acquire()
            reused = self.acquire()
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(same_content_manifest)}):
            same_content = self.acquire('task-two')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(second_manifest)}):
            second = self.acquire('task-three')
        self.assertTrue(reused['reused'])
        self.assertEqual(first['build_run'], reused['build_run'])
        self.assertTrue(same_content['reused'])
        self.assertEqual(first['build_run'], same_content['build_run'])
        self.assertNotEqual(first['build_run'], second['build_run'])
        self.start(first)

    def test_changing_explicit_cpa_manifest_in_place_invalidates_reuse(self):
        manifest = self.root / 'private-cpa.json'
        manifest.write_text('{"fixture":"before"}\n')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}):
            first = self.acquire()
            manifest.write_text('{"fixture":"after"}\n')
            second = self.acquire('task-two')
        self.assertNotEqual(first['build_run'], second['build_run'])
        self.start(first)

    def test_missing_or_invalid_cpa_manifest_fails_before_build(self):
        missing = self.root / 'missing-cpa.json'
        directory = self.root / 'cpa-directory'
        directory.mkdir()
        invalid = []
        for index, contents in enumerate((b'not json\n', b'[]', b'\xff',
                                           b'{"size":NaN}', b'{"size":Infinity}', b'{"size":-Infinity}')):
            path = self.root / ('invalid-cpa-%s.json' % index)
            path.write_bytes(contents)
            invalid.append(path)
        fifo = self.root / 'cpa-fifo'
        os.mkfifo(fifo)
        link = self.root / 'cpa-link.json'
        link.symlink_to(self.repo / 'apps/desktop/src-tauri/development-cpa-artifacts.v1.json')
        for manifest in (missing, directory, *invalid, fifo, link):
            with self.subTest(manifest=manifest), \
                    patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}), \
                    patch.object(m.local, 'run') as build, \
                    self.assertRaisesRegex(ValueError, 'CPA manifest'):
                self.acquire()
            build.assert_not_called()

    def test_source_changes_do_not_invalidate_an_acquired_build_or_publish_its_path(self):
        manifest = self.root / 'private-cpa.json'
        manifest.write_text('{"fixture":"stable"}\n')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}):
            result = self.acquire()
        row = self.store.load(result['build_run'])
        for public in (result, row, self.manager.locate(result['lease'])):
            self.assertNotIn(str(manifest), json.dumps(public, sort_keys=True))
        snapshot = self.store.record_path(result['build_run']).parent / 'cpa-manifest.json'
        self.assertEqual(snapshot.stat().st_mode & 0o777, 0o600)
        self.assertFalse((self.manager.root / 'manifests').exists())
        manifest.write_text('{"fixture":"changed"}\n')
        self.start(result)
        self.assertEqual(snapshot.read_bytes(), b'{"fixture":"stable"}\n')

    def test_default_cpa_manifest_comes_from_candidate_and_reuses_equal_explicit_bytes(self):
        manifest = self.repo / 'apps/desktop/src-tauri/development-cpa-artifacts.v1.json'
        original = manifest.read_bytes()
        manifest.write_text('{"fixture":"dirty execution checkout"}\n')
        result = self.acquire()
        self.assertEqual((Path(result['checkout']) / 'target/debug/compiled-cpa.json').read_bytes(), original)
        manifest.write_bytes(original)
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}):
            explicit = self.acquire('task-two')
        self.assertTrue(explicit['reused'])
        self.assertEqual(result['build_run'], explicit['build_run'])

    def test_relative_cpa_manifest_uses_desktop_package_directory(self):
        manifest = self.repo / 'apps/desktop/src-tauri/relative-cpa.json'
        manifest.write_text('{"fixture":"relative"}\n')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': 'relative-cpa.json'}):
            result = self.acquire()
            reused = self.acquire('task-two')
        self.assertTrue(reused['reused'])
        self.assertEqual((Path(result['checkout']) / 'target/debug/compiled-cpa.json').read_bytes(), manifest.read_bytes())
        self.start(result)

    def test_deleted_source_rejects_fresh_acquisition_but_old_lease_still_starts(self):
        manifest = self.root / 'private-cpa.json'
        manifest.write_text('{"fixture":"stable"}\n')
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}):
            result = self.acquire()
            manifest.unlink()
            with patch.object(m.local, 'run') as build, self.assertRaisesRegex(ValueError, 'CPA manifest'):
                self.acquire('task-two')
            build.assert_not_called()
        self.start(result)

    def test_native_build_consumes_captured_snapshot_when_source_changes_during_compilation(self):
        manifest = self.root / 'private-cpa.json'
        original = b'{"fixture":"verified"}\n'
        manifest.write_bytes(original)
        start_command = m.local.remote.start_locked_command
        snapshot_paths = []

        def changed_source(argv, checkout, env, output, handles):
            if argv != m.COMMAND:
                return start_command(argv, checkout, env, output, handles)
            manifest.write_text('{"fixture":"changed during compilation"}\n')
            source = Path(env['HIROUTE_CPA_MANIFEST'])
            snapshot_paths.append(source)
            return start_command(argv, checkout, env, output, handles)

        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}), \
                patch.object(m.local.remote, 'start_locked_command', side_effect=changed_source):
            result = self.acquire()
        self.assertEqual((Path(result['checkout']) / 'target/debug/compiled-cpa.json').read_bytes(), original)
        self.assertEqual(len(snapshot_paths), 1)
        self.assertNotEqual(snapshot_paths[0], manifest)
        self.assertEqual(snapshot_paths[0].stat().st_mode & 0o777, 0o600)
        self.start(result)

    def test_original_cpa_manifest_is_read_only_once_before_build_preparation(self):
        manifest = self.root / 'private-cpa.json'
        original = b'{"fixture":"captured"}\n'
        manifest.write_bytes(original)
        run = m.local.run

        def change_before_prepare(*args, **kwargs):
            manifest.write_text('{"fixture":"new input"}\n')
            return run(*args, **kwargs)

        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}), \
                patch.object(m, 'read_cpa_manifest', wraps=m.read_cpa_manifest) as read, \
                patch.object(m.local, 'run', side_effect=change_before_prepare):
            result = self.acquire()
        source_reads = [call for call in read.call_args_list if Path(call.args[0]) == manifest]
        self.assertEqual(len(source_reads), 1)
        self.assertEqual((Path(result['checkout']) / 'target/debug/compiled-cpa.json').read_bytes(), original)

    def test_hardlinked_regular_manifests_are_ingested_by_content(self):
        manifest = self.root / 'private-cpa.json'
        manifest.write_text('{"fixture":"hardlinked"}\n')
        alias = self.root / 'other-cpa.json'
        os.link(manifest, alias)
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}):
            first = self.acquire()
        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(alias)}):
            second = self.acquire('task-two')
        self.assertTrue(second['reused'])
        self.assertEqual(first['build_run'], second['build_run'])

    def test_altered_cpa_snapshot_rejects_reuse_and_start(self):
        result = self.acquire()
        snapshot = self.store.record_path(result['build_run']).parent / 'cpa-manifest.json'
        snapshot.write_text('{"fixture":"altered snapshot"}\n')
        with self.assertRaisesRegex(ValueError, 'CPA manifest snapshot'):
            self.acquire('task-two')
        with patch.object(m.pilot, 'start') as launch, self.assertRaisesRegex(ValueError, 'CPA manifest snapshot'):
            self.manager.start(types.SimpleNamespace(lease=result['lease'], case='model', timeout=1,
                                                     data_root=None, process_home=None))
        launch.assert_not_called()

    def real_consumer(self):
        if not REAL_RUSTC:
            raise RuntimeError('The required real-consumer case needs rustc; it cannot be skipped')
        native = Path(__file__).resolve().parents[1] / 'apps/desktop/src-tauri'
        manifest = self.root / 'private-cpa.json'
        original = b'{"fixture":"captured by real consumer"}\n'
        manifest.write_bytes(original)
        output = self.root / 'consumer-out'
        output.mkdir()
        build_script = self.root / 'build-script'
        consumer = self.root / 'manifest-consumer'
        source = self.root / 'consumer.rs'
        # Reuse the production include_str! declaration, not another manifest reader.
        declaration = next(line for line in (native / 'src/development_cpa.rs').read_text().splitlines()
                           if line.startswith('const MANIFEST_JSON:'))
        source.write_text(declaration + '\nfn main() { print!("{}", MANIFEST_JSON); }\n')
        start_command = m.local.remote.start_locked_command
        observed = []

        def compile_snapshot(argv, checkout, env, log, handles):
            if argv == m.COMMAND:
                manifest.unlink()  # The consumer must no longer need the ingestion source.
                consumer_env = dict(env, PATH=REAL_PATH, OUT_DIR=str(output), PROFILE='debug')
                consumer_step('build-script-compile',
                              [REAL_RUSTC, '--edition=2024', str(native / 'build.rs'), '-o', str(build_script)],
                              native, consumer_env, 60)
                consumer_step('build-script-execute', [str(build_script)], native, consumer_env, 10)
                consumer_step('embedding-compile',
                              [REAL_RUSTC, '--edition=2024', str(source), '-o', str(consumer)],
                              native, consumer_env, 60)
                observed.append(consumer_step('embedding-execute', [str(consumer)], native, consumer_env, 10))
            return start_command(argv, checkout, env, log, handles)

        with patch.dict(os.environ, {'HIROUTE_CPA_MANIFEST': str(manifest)}), \
                patch.object(m.local.remote, 'start_locked_command', side_effect=compile_snapshot):
            result = self.acquire()
        self.assertEqual(observed, [original])
        self.assertEqual((output / 'cpa-artifacts.json').read_bytes(), original)
        self.start(result)

    def test_failed_build_is_retained_without_creating_duplicate_on_retry(self):
        cargo = self.bin / 'cargo'
        source = cargo.read_text()
        cargo.write_text(source.replace("root=pathlib.Path.cwd()", "\nif name=='cargo' and 'build' in sys.argv: sys.exit(4)\nroot=pathlib.Path.cwd()"))
        with self.assertRaisesRegex(ValueError, 'build failed/unknown'):
            self.acquire()
        with self.assertRaisesRegex(ValueError, 'Previous build failed/interrupted'):
            self.acquire()
        self.assertEqual(len(list((self.store.root / 'checkouts').iterdir())), 1)

    def test_interrupted_acquisition_is_indexed_before_compilation(self):
        recipe = m.build_recipe(self.repo, self.sha, 2)
        key = m.digest(recipe)
        def disconnected(row):
            m.local.remote.save(self.manager.index(key), {'run': row['id'], 'key': key})
            raise RuntimeError('simulated disconnect')
        args = types.SimpleNamespace(repo=str(self.repo), ref=self.branch, sha=self.sha, source='origin',
                                     jobs=2, command=m.COMMAND, keep=True, cargo_only=False, timeout=20)
        with self.assertRaisesRegex(RuntimeError, 'simulated disconnect'):
            m.local.run(self.store, args, on_record=disconnected)
        with self.assertRaisesRegex(ValueError, 'Previous build failed/interrupted'):
            self.acquire()
        self.assertEqual(len(list((self.store.root / 'runs').iterdir())), 1)
        self.assertEqual(len(list((self.store.root / 'checkouts').iterdir())), 0)

    def test_native_platform_preflight_does_not_register_an_unfinishable_session(self):
        result = self.acquire()
        args = types.SimpleNamespace(lease=result['lease'], case='model', timeout=1,
                                     data_root=None, process_home=None, native_app=True)
        with patch.object(m.sys, 'platform', 'linux'), patch.object(m.pilot, 'start') as launch:
            with self.assertRaisesRegex(ValueError, 'requires macOS'):
                self.manager.start(args)
            launch.assert_not_called()
        owner = self.manager.locate(result['lease'])['pilot_leases'][result['lease']]
        self.assertEqual(owner['sessions'], {})
        self.assertEqual(self.finish(result)['cleanup'], 'removed')

    def test_native_cpa_preflight_does_not_register_an_unfinishable_session(self):
        result = self.acquire()
        sidecar = Path(result['checkout']) / 'target/debug/cliproxyapi'
        sidecar.write_text('unattested fixture sidecar')
        sidecar.chmod(0o700)
        args = types.SimpleNamespace(lease=result['lease'], case='model', timeout=1,
                                     data_root=None, process_home=None, native_app=True)
        with patch.object(m.sys, 'platform', 'darwin'), patch.object(m.pilot, 'start') as launch:
            with self.assertRaisesRegex(ValueError, 'no unique native target'):
                self.manager.start(args)
            launch.assert_not_called()
        owner = self.manager.locate(result['lease'])['pilot_leases'][result['lease']]
        self.assertEqual(owner['sessions'], {})
        self.assertEqual(self.finish(result)['cleanup'], 'removed')

    def test_raw_pilot_start_cannot_create_unregistered_session(self):
        result = self.acquire()
        args = m.pilot.build_parser().parse_args(['start', '--build-run', result['build_run'],
                                                '--app', str(Path(result['checkout']) / 'target/debug/hiroute-desktop')])
        with patch.object(m.pilot.subprocess, 'Popen') as launch:
            # Verification itself uses subprocess.check_output, so exercise the
            # validated attestation boundary without mocking Git internals.
            with patch.object(m.pilot, 'verify_managed_pilot_build', return_value={'reuse_managed': True}):
                with self.assertRaisesRegex(ValueError, 'owner lease'):
                    m.pilot.start(args)
            launch.assert_not_called()

    def test_altered_binary_rejects_reuse_without_silent_rebuild(self):
        first = self.acquire()
        (Path(first['checkout']) / 'target/debug/hiroute').write_text('replaced')
        with self.assertRaisesRegex(ValueError, 'executable identity'):
            self.acquire('task-two')
        self.assertEqual(len(list((self.store.root / 'checkouts').iterdir())), 1)

    def test_altered_frontend_rejects_reuse(self):
        first = self.acquire()
        row = self.store.load(first['build_run'])
        (Path(row['pilot_frontend']['path']) / 'index.html').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'frontend content'):
            self.acquire('task-two')

    def test_frontend_links_are_refused(self):
        first = self.acquire()
        row = self.store.load(first['build_run'])
        (Path(row['pilot_frontend']['path']) / 'link').symlink_to(self.repo / '.gitignore')
        with self.assertRaisesRegex(ValueError, 'link'):
            self.acquire('task-two')

    def test_last_owner_cleans_binaries_and_preserves_evidence_and_sessions(self):
        first = self.acquire()
        second = self.acquire('task-two')
        session = self.start(first)
        retained = self.finish(first, ('model=green',))
        self.assertEqual(retained['cleanup'], 'retained')
        self.assertTrue(Path(first['checkout']).exists())
        removed = self.finish(second)
        self.assertEqual(removed['cleanup'], 'removed')
        self.assertEqual(removed['scenario'], 'unassessed')
        self.assertFalse(Path(first['checkout']).exists())
        self.assertTrue(Path(session['session']).exists())
        run = self.store.record_path(first['build_run']).parent
        self.assertTrue((run / 'artifacts/evidence/result').exists())
        self.assertTrue((run / 'frontend-dist/index.html').exists())
        self.assertTrue((run / 'command.log').exists())
        self.assertEqual(self.finish(second)['cleanup'], 'removed')

    def test_all_executed_green_cases_can_finish_green(self):
        result = self.acquire(cases=('model', 'agent'))
        self.start(result, 'model')
        self.start(result, 'agent')
        final = self.finish(result, ('model=green', 'agent=green'))
        self.assertEqual(final['scenario'], 'green')

    def test_missing_unstarted_duplicate_assessments_do_not_release(self):
        result = self.acquire(cases=('model', 'agent'))
        for values in [('model=red',), ('model=green', 'agent=green'), ('model=red', 'model=green')]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                self.finish(result, values)
        self.assertEqual(self.manager.locate(result['lease'])['pilot_leases'][result['lease']]['state'], 'open')

    def test_unknown_process_ownership_retains_open_lease_and_build(self):
        result = self.acquire()
        self.start(result)
        with self.assertRaisesRegex(ValueError, 'PID reused'):
            self.finish(result, ('model=red',), stop_error=ValueError('PID reused'))
        self.assertTrue(Path(result['checkout']).exists())
        self.assertEqual(self.manager.locate(result['lease'])['pilot_leases'][result['lease']]['state'], 'open')

    def test_wrong_diagnostic_level_does_not_claim_green(self):
        result = self.acquire()
        self.start(result)
        with self.assertRaisesRegex(ValueError, 'actual Debug'):
            self.finish(result, ('model=green',), debug=False)
        self.assertTrue(Path(result['checkout']).exists())

    def test_source_changes_refuse_cleanup_and_preserve_original_verdict(self):
        result = self.acquire()
        (Path(result['checkout']) / 'notes').write_text('unique evidence')
        finished = self.finish(result)
        self.assertEqual(finished['cleanup'], 'retained')
        self.assertTrue(Path(result['checkout']).exists())
        with self.assertRaisesRegex(ValueError, 'cannot be rewritten'):
            self.finish(result, ('model=red',))

    def test_raw_release_cannot_cleanup_open_pilot_lease(self):
        result = self.acquire()
        row = self.store.load(result['build_run'])
        row.update(keep=False, evidence_saved=True)
        self.store.save(row)
        self.assertEqual(self.store.preview_one(row)['state'], 'skipped')
        with self.assertRaisesRegex(ValueError, 'Pilot owners'):
            self.store.apply(row['id'], 'not-a-token')

    def test_interrupted_start_never_guesses_that_processes_are_absent(self):
        result = self.acquire()
        with patch.object(m.pilot, 'start', side_effect=RuntimeError('interrupted')):
            with self.assertRaisesRegex(RuntimeError, 'interrupted'):
                self.manager.start(types.SimpleNamespace(lease=result['lease'], case='model', timeout=1,
                                                        data_root=None, process_home=None))
        with self.assertRaises(OSError):
            self.finish(result, ('model=red',))
        self.assertTrue(Path(result['checkout']).exists())

    def test_same_key_in_flight_refuses_second_acquisition(self):
        result = self.acquire()
        row = self.manager.locate(result['lease'])
        with self.manager.locked(row['pilot_reuse']['key']):
            with self.assertRaises(BlockingIOError):
                self.acquire('task-two')


class ConsumerRouting(unittest.TestCase):
    def test_default_discovery_does_not_select_the_real_consumer(self):
        self.assertNotIn('test_production_build_script_and_embedded_constant_consume_the_captured_snapshot',
                         unittest.defaultTestLoader.getTestCaseNames(Lifecycle))
        self.assertNotIn('real_consumer', unittest.defaultTestLoader.getTestCaseNames(Lifecycle))

    def test_required_consumer_fails_for_zero_skipped_or_failed_cases(self):
        for count, skipped, successful in ((0, [], True), (2, [], True),
                                           (1, [('case', 'skip')], True), (1, [], False), (1, [], True)):
            result = types.SimpleNamespace(testsRun=count, skipped=skipped, wasSuccessful=lambda: successful)
            with self.subTest(count=count, skipped=skipped, successful=successful), \
                    patch.object(unittest.TextTestRunner, 'run', return_value=result):
                self.assertEqual(run_real_consumer(), 0 if count == 1 and not skipped and successful else 1)

    def test_missing_real_compiler_is_an_error_not_a_skip(self):
        with patch.dict(Lifecycle.real_consumer.__globals__, {'REAL_RUSTC': None}):
            with self.assertRaisesRegex(RuntimeError, 'needs rustc'):
                Lifecycle('real_consumer').real_consumer()

    def test_failed_subprocess_diagnostics_survive_fixture_cleanup(self):
        outcomes = (
            (subprocess.CompletedProcess(['synthetic-consumer'], -9, b'partial output', b'synthetic rejection'),
             -9, 'partial output', 'synthetic rejection'),
            (subprocess.TimeoutExpired(['synthetic-consumer'], 10, output=b'partial output', stderr=b'timed out'),
             None, 'partial output', 'timed out'),
            (OSError('synthetic launch failure'), None, '', ''),
        )
        for outcome, returncode, stdout, stderr in outcomes:
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as root:
                evidence = Path(root) / 'evidence'
                with tempfile.TemporaryDirectory(dir=root) as fixture:
                    with patch.dict(consumer_step.__globals__, {'CONSUMER_EVIDENCE': evidence}), \
                            patch.object(subprocess, 'run', side_effect=[outcome]) as run, \
                            self.assertRaisesRegex(RuntimeError, 'build-script-execute'):
                        consumer_step('build-script-execute', ['synthetic-consumer'], fixture, {}, 10)
                    run.assert_called_once()
                report = json.loads((evidence / 'build-script-execute.json').read_text())
                self.assertEqual(report['phase'], 'build-script-execute')
                self.assertEqual(report['returncode'], returncode)
                self.assertEqual(report['stdout'], stdout)
                self.assertEqual(report['stderr'], stderr)
                if isinstance(outcome, Exception):
                    self.assertEqual(report['error'], str(outcome))


def run_real_consumer():
    result = unittest.TextTestRunner(verbosity=2).run(unittest.TestSuite([Lifecycle('real_consumer')]))
    return 0 if result.wasSuccessful() and result.testsRun == 1 and not result.skipped else 1


if __name__ == '__main__':
    if sys.argv[1:] == ['--real-consumer']:
        sys.exit(run_real_consumer())
    unittest.main()
