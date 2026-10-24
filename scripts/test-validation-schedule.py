#!/usr/bin/env python3
"""Prove phase isolation, exact coverage and shared-slot execution without Rust."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner = load('remote-rust')
scheduler = load('validation-schedule')
reporting = load('validation-report')
COMMAND = ['cargo', 'test', '--locked', '--workspace', '--exclude', 'hiroute-desktop', '--all-features']
TEST_BINARY = '''import os,sys,time,pathlib
root=pathlib.Path(__file__).resolve().parents[3]
assert sys.argv[1:] == ['default_two_domain_smoke', '--exact']
assert os.environ['HIROUTE_SMOKE_REQUIRE_PREPARED']=='1'
assert os.environ['CARGO_PKG_NAME']=='hiroute-product-e2e'
assert os.environ['CARGO_BIN_EXE_hiroute-smoke']=='/fixture/hiroute-smoke'
assert os.environ['RUST_RECURSION_COUNT']=='1'
assert (root/'prepared').exists()
(root/'smoke.ready').touch()
until=time.monotonic()+5
while not (root/'remainder.ready').exists():
 if time.monotonic()>until: sys.exit(98)
 time.sleep(.01)
bad=os.environ['FIXTURE_MODE']=='red-smoke'
print('running 1 test')
print('test default_two_domain_smoke ... '+('FAILED' if bad else 'ok'))
print('test result: '+('FAILED' if bad else 'ok')+f'. {int(not bad)} passed; {int(bad)} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
sys.exit(101 if bad else 0)
'''


class Schedule(unittest.TestCase):
    def run_dag_isolation_fixture(self, fail_cpa=False, fail_fast=True, omit_cpa=False,
                                  fail_observation=False, omit_observation=False,
                                  duplicate_observation=False, preparation_failure=None):
        """Exercise the real DAG controller with bounded fake build/target processes."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / 'run'
            run.mkdir()
            names = ['hiroute_cpa_bridge', 'hiroute_observation', 'hiroute_gateway', 'smoke_cli']
            if omit_cpa:
                names.remove('hiroute_cpa_bridge')
            if omit_observation:
                names.remove('hiroute_observation')
            if duplicate_observation:
                names.append('hiroute_observation')
            targets = [dict(name=name, package=name.replace('_', '-'), source=name,
                            cwd=str(root), test_source='src/lib.rs', label=name)
                       for name in names]
            metadata = {'packages': [dict(id=row['package'], name=row['package'])
                                     for row in targets]}
            cpa_done = threading.Event()
            observation_done = threading.Event()
            admitted = []
            ordinary_done = threading.Event()

            class FixtureRunner:
                @staticmethod
                def save(path, data):
                    Path(path).write_text(json.dumps(data))
                @staticmethod
                def command(_args, _cwd):
                    return json.dumps(metadata)
                @staticmethod
                def reporting():
                    return reporting
                @staticmethod
                def test_summary(_text, _command):
                    return dict(counts=dict(passed=1, failed=0, ignored=0))
                @staticmethod
                def start_locked_command(args, _cwd, _env, output, _handles):
                    if '--no-run' not in args:
                        self.assertTrue(cpa_done.is_set(), 'CPA must finish before preparation/doctests')
                        self.assertTrue(observation_done.is_set(),
                                        'Observation deadlines must not overlap preparation/doctests')
                    if '--doc' in args:
                        output.write('   Doc-tests fixture\nrunning 1 test\n'
                                     'test fixture_doc ... ok\n'
                                     'test result: ok. 1 passed; 0 failed; 0 ignored; '
                                     '0 measured; 0 filtered out; finished in 0.01s\n')
                    return SimpleNamespace(wait=lambda timeout: 0)

            def run_targets(_runner, _root, _run, selected, *_args, **kwargs):
                results = []
                for item in selected:
                    name = item['name']
                    if name == 'hiroute_cpa_bridge':
                        self.assertFalse({row['name'] for row in selected}
                                         - {'hiroute_cpa_bridge', 'hiroute_observation'},
                                         'Isolated libraries must not share other target processes')
                        self.assertEqual(kwargs['workers'], 1,
                                         'The two isolated libraries must not overlap')
                        self.assertEqual(item.get('test_threads'), 1,
                                         'CPA port handoffs must not race inside its binary')
                        cpa_done.set()
                    elif name == 'hiroute_observation':
                        self.assertTrue(cpa_done.is_set())
                        self.assertEqual(kwargs['workers'], 1)
                        self.assertEqual(item.get('test_threads'), 8,
                                         'Keep the complete library at its existing parallelism')
                        observation_done.set()
                    else:
                        self.assertTrue(cpa_done.is_set(), 'No runtime target may overlap CPA')
                        self.assertTrue(observation_done.is_set(),
                                        'No other runtime target may overlap Observation')
                    admitted.append(name)
                    failed = ((fail_cpa and name == 'hiroute_cpa_bridge')
                              or (fail_observation and name == 'hiroute_observation'))
                    path = run / ('fixture-' + name + '.log')
                    path.write_text(name + '\n')
                    results.append(dict(name=name, index=item['index'], log=str(path), complete=not failed,
                                        process_exit=int(failed),
                                        counts=dict(passed=int(not failed), failed=int(failed), ignored=0)))
                    stop_event = kwargs.get('stop_event', _args[-1] if _args else None)
                    if failed and fail_fast and stop_event is not None:
                        stop_event.set()
                        break
                if any(item['name'] == 'hiroute_gateway' for item in selected):
                    ordinary_done.set()
                return results

            def prepare(*_args):
                if preparation_failure:
                    self.assertTrue(ordinary_done.wait(5), 'Independent target must finish first')
                    raise ValueError(preparation_failure)
                return {}

            mocks = dict(dag_catalog=lambda *_args: targets,
                         dag_list=lambda source, *_args: ['default_two_domain_smoke' if source == 'smoke_cli' else source],
                         dag_snapshot=lambda *_args: None,
                         dag_daemon_fixture=lambda *_args: {},
                         dag_bind_daemon_fixture=lambda *_args: None,
                         dag_bind_metadata=lambda *_args: None,
                         dag_toolchain_context=lambda *_args: {'path': '/fixture/toolchain'},
                         dag_products=prepare,
                         dag_publication_daemon=lambda *_args: root,
                         dag_run_targets=run_targets)
            output = io.StringIO()
            with contextlib.ExitStack() as stack:
                for name, replacement in mocks.items():
                    stack.enter_context(patch.object(scheduler, name, side_effect=replacement))
                command = COMMAND + ([] if fail_fast else ['--no-fail-fast'])
                result = scheduler.execute_dag(FixtureRunner, root, run, root,
                                               dict(command=command, timeout=15, sha='a'*40),
                                               None, output, reserved_handles=[])
            return result, admitted, output.getvalue()

    def test_cpa_isolation_keeps_every_target_and_case(self):
        result, admitted, _ = self.run_dag_isolation_fixture()
        self.assertTrue(result['complete'], result)
        self.assertEqual(admitted, ['hiroute_cpa_bridge', 'hiroute_observation',
                                    'hiroute_gateway', 'smoke_cli'])
        self.assertEqual(result['expected_tests'], 4)
        self.assertEqual(result['executed_targets'], 4)
        self.assertEqual(result['counts'], dict(passed=5, failed=0, ignored=0))

    def test_preparation_failure_preserves_completed_targets_and_unexecuted_coverage(self):
        for fail_fast in (True, False):
            with self.subTest(fail_fast=fail_fast):
                result, admitted, log = self.run_dag_isolation_fixture(
                    preparation_failure='selected receipt changed', fail_fast=fail_fast)
                self.assertFalse(result['complete'])
                self.assertEqual(result['process_exit'], 1)
                self.assertEqual(result['error'], 'selected receipt changed')
                self.assertEqual(admitted, ['hiroute_cpa_bridge', 'hiroute_observation',
                                            'hiroute_gateway'])
                self.assertEqual(result['executed_targets'], 3)
                self.assertEqual(len(result['target_results']), 3)
                self.assertEqual(result['counts'], dict(passed=3, failed=0, ignored=0))
                self.assertEqual([row['name'] for row in result['unexecuted_targets']], ['smoke_cli'])
                self.assertEqual(result['unexecuted_targets'][0]['cases'], ['default_two_domain_smoke'])
                self.assertFalse(result['doctests_executed'])
                self.assertIn('hiroute_gateway', log)
                summary = runner.scheduled_test_summary(log, COMMAND, result)
                self.assertEqual(summary['counts']['passed'], 3)
                self.assertNotEqual(summary['state'], 'rust_tests_passed')

    def test_observation_deadline_isolation_preserves_parallelism_and_all_targets(self):
        result, admitted, _ = self.run_dag_isolation_fixture()
        self.assertTrue(result['complete'], result)
        self.assertEqual(admitted.index('hiroute_observation'), 1)
        self.assertEqual(result['expected_tests'], 4)
        self.assertEqual(result['executed_targets'], 4)

    def test_failed_isolated_target_obeys_fail_fast_and_keeps_its_log(self):
        result, admitted, log = self.run_dag_isolation_fixture(fail_cpa=True)
        self.assertFalse(result['complete'])
        self.assertEqual(admitted, ['hiroute_cpa_bridge'])
        self.assertEqual(result['executed_targets'], 1)
        self.assertIn('hiroute_cpa_bridge', log)
        result, admitted, _ = self.run_dag_isolation_fixture(fail_cpa=True, fail_fast=False)
        self.assertFalse(result['complete'])
        self.assertEqual(len(admitted), 4)
        self.assertEqual(result['counts']['failed'], 1)

    def test_failed_observation_target_keeps_failure_and_blocks_preparation(self):
        result, admitted, log = self.run_dag_isolation_fixture(fail_observation=True)
        self.assertFalse(result['complete'])
        self.assertEqual(admitted, ['hiroute_cpa_bridge', 'hiroute_observation'])
        self.assertEqual(result['executed_targets'], 2)
        self.assertIn('hiroute_observation', log)
        result, admitted, _ = self.run_dag_isolation_fixture(
            fail_observation=True, fail_fast=False)
        self.assertFalse(result['complete'])
        self.assertEqual(len(admitted), 4)
        self.assertEqual(result['counts']['failed'], 1)

    def test_missing_cpa_isolation_target_cannot_silently_narrow_the_schedule(self):
        result, admitted, _ = self.run_dag_isolation_fixture(omit_cpa=True)
        self.assertFalse(result['complete'])
        self.assertEqual(admitted, [])
        self.assertIn('hiroute-cpa-bridge library target', result['error'])

    def test_missing_observation_target_cannot_silently_narrow_the_schedule(self):
        result, admitted, _ = self.run_dag_isolation_fixture(omit_observation=True)
        self.assertFalse(result['complete'])
        self.assertEqual(admitted, [])
        self.assertIn('hiroute-observation library target', result['error'])

    def test_duplicate_observation_target_cannot_duplicate_coverage(self):
        result, admitted, _ = self.run_dag_isolation_fixture(duplicate_observation=True)
        self.assertFalse(result['complete'])
        self.assertEqual(admitted, [])
        self.assertIn('hiroute-observation library target', result['error'])

    def test_isolated_binary_runs_all_cases_with_one_thread_and_exec_audit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / 'fixture'
            executable.write_bytes(b'compiled fixture')
            item = dict(name='hiroute_cpa_bridge', package_name='hiroute-cpa-bridge',
                        executable=str(executable), sha256=scheduler.file_digest(executable),
                        index=0, label='src/lib.rs', cwd=str(root), test_threads=1,
                        cases=['case_one', 'case_two'])
            class FixtureRunner:
                @staticmethod
                def reporting():
                    return reporting
                @staticmethod
                def save(path, data):
                    Path(path).write_text(json.dumps(data))
                @staticmethod
                def start_locked_command(args, _cwd, _env, output, _handles):
                    self.assertEqual(args[-2:], [str(executable), '--test-threads=1'])
                    trace = Path(args[args.index('-o') + 1])
                    trace.write_text('123 execve("/fixture/test", [...], 0x1) = 0\n')
                    output.write('running 2 tests\ntest case_one ... ok\n'
                                 'test case_two ... ignored, fixture\n'
                                 'test result: ok. 1 passed; 0 failed; 1 ignored; '
                                 '0 measured; 0 filtered out; finished in 0.01s\n')
                    return SimpleNamespace(wait=lambda timeout: 0)
            with patch.object(scheduler.sys, 'platform', 'linux'), \
                 patch.object(scheduler.shutil, 'which', return_value='/fixture/strace'):
                results = scheduler.dag_run_targets(
                    FixtureRunner, root, root, [item], root, None, {},
                    scheduler.time.monotonic() + 5, workers=1, reserved_handles=[])
            self.assertEqual(len(results), 1)
            self.assertTrue(results[0]['complete'], results)
            self.assertEqual(results[0]['counts'], dict(passed=1, failed=0, ignored=1))
            self.assertEqual(results[0]['exec_audit']['cargo_rustc_starts'], 0)
            self.assertLessEqual(results[0]['started_at'], results[0]['finished_at'])

    def test_gateway_runtime_consumers_wait_for_prepared_products(self):
        tests = Path(__file__).resolve().parents[1] / 'tools/e2e-harness/tests'
        consumers = {source.stem for source in tests.glob('*.rs')
                     if 'mod runtime_support;' in source.read_text()}
        self.assertTrue(consumers, 'Gateway runtime consumer inventory must be nonempty')
        self.assertIn('p0_gateway_native_sessions', consumers)
        self.assertFalse(consumers - scheduler.PRODUCT_TARGETS,
                         'Gateway runtime consumers must wait for prepared products: '
                         + ', '.join(sorted(consumers - scheduler.PRODUCT_TARGETS)))

    def test_exec_audit_rejects_runtime_cargo_without_storing_arguments(self):
        trace = ('123 execve("/tmp/bin/test", [...], 0x1) = 0\n'
                 '124 execve("/home/user/.cargo/bin/cargo", [...], 0x1) = 0\n'
                 '125 execveat(AT_FDCWD, "/toolchain/bin/rustc", [...], 0) = 0\n')
        audit = scheduler.dag_exec_audit(trace)
        self.assertEqual(audit['process_starts'], 3)
        self.assertEqual(audit['cargo_rustc_starts'], 2)
        self.assertEqual(audit['blocked_classes'], ['cargo', 'rustc'])
        self.assertNotIn('/home/user', json.dumps(audit))
        with self.assertRaisesRegex(ValueError, 'no started'):
            scheduler.dag_exec_audit('')

    def test_metadata_is_bound_to_both_dependency_gate_targets(self):
        artifact = {'path': '/fixture/dag-cargo-metadata.json', 'sha256': 'digest'}
        targets = [{'name': name, 'package_name': 'hiroute-product-e2e'}
                   for name in ('product_oracle', 'transaction_recovery')]
        scheduler.dag_bind_metadata(targets, artifact)
        for row in targets:
            self.assertEqual(row['environment']['HIROUTE_VALIDATION_METADATA_JSON'], artifact['path'])
            self.assertEqual(row['external_artifacts'], [artifact])
        with self.assertRaisesRegex(ValueError, 'consumer set changed'):
            scheduler.dag_bind_metadata(targets[:1], artifact)

    def test_toolchain_manifest_is_candidate_bound_and_requires_host(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / 'cargo'
            rustc = root / 'rustc'
            cargo.write_bytes(b'cargo tool')
            rustc.write_bytes(b'rustc tool')
            class Runner:
                @staticmethod
                def command(args, cwd):
                    self.assertEqual(args, ['rustup', 'which', 'cargo'])
                    return str(cargo)
                @staticmethod
                def save(path, data):
                    Path(path).write_text(json.dumps(data))
            def version(args, **_kwargs):
                return ('cargo 1.97.1\n' if args[0] == str(cargo)
                        else 'rustc 1.97.1\nhost: x86_64-unknown-linux-gnu\n')
            with patch.object(scheduler.subprocess, 'check_output', side_effect=version), \
                 patch.object(scheduler.shutil, 'which', return_value=None):
                artifact = scheduler.dag_toolchain_context(Runner, root, 'a'*40, root)
            record = json.loads(Path(artifact['path']).read_text())
            self.assertEqual(record['sha'], 'a'*40)
            self.assertEqual(record['target_triple'], 'x86_64-unknown-linux-gnu')
            self.assertEqual(scheduler.file_digest(Path(artifact['path'])), artifact['sha256'])
            with patch.object(scheduler.subprocess, 'check_output', return_value='rustc 1.97.1\n'), \
                 patch.object(scheduler.shutil, 'which', return_value=None), \
                 self.assertRaisesRegex(ValueError, 'host target'):
                scheduler.dag_toolchain_context(Runner, root, 'a'*40, root)

    def test_daemon_fixture_survives_gateway_overwrite_and_binds_exact_consumers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'target/debug/hirouted'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'daemon with integration hooks')
            metadata = {'packages': [{'id': 'daemon-id', 'name': 'hiroute-daemon'}]}
            event = {'reason': 'compiler-artifact', 'package_id': 'daemon-id',
                     'target': {'name': 'hirouted', 'kind': ['bin']},
                     'profile': {'test': False}, 'features': ['integration-test-hooks'],
                     'executable': str(binary)}
            artifact = scheduler.dag_daemon_fixture(json.dumps(event), root, metadata, 'fixture')
            binary.write_bytes(b'gateway with the same output name')
            self.assertEqual(Path(artifact['path']).read_bytes(), b'daemon with integration hooks')
            self.assertEqual(scheduler.file_digest(Path(artifact['path'])), artifact['sha256'])
            tests = root / 'crates/daemon/tests'
            tests.mkdir(parents=True)
            targets = []
            for name in ('discovered_model_product', 'subscription_management_product'):
                (tests / (name + '.rs')).write_text(
                    '#[path = "support/discovered_model_product_support.rs"] mod product_support;')
                targets.append({'name': name, 'test_source': 'tests/' + name + '.rs',
                                'cwd': str(tests.parent), 'package_name': 'hiroute-daemon'})
            scheduler.dag_bind_daemon_fixture(targets, artifact)
            self.assertTrue(all(row['environment']['HIROUTE_VALIDATION_DAEMON_BIN']
                                == artifact['path'] for row in targets))
            self.assertTrue(all(row['external_artifacts'] == [artifact] for row in targets))
            event['features'] = []
            with self.assertRaisesRegex(ValueError, 'feature-qualified'):
                scheduler.dag_daemon_fixture(json.dumps(event), root, metadata, 'second')
            with self.assertRaisesRegex(ValueError, 'consumer set changed'):
                scheduler.dag_bind_daemon_fixture(targets[:1], artifact)

    def test_dag_phase_records_render_in_validation_report(self):
        phase = {'name': 'compile', 'status': 'completed', 'process_exit': 0,
                 'build_jobs': 64, 'started_at': 1.0, 'finished_at': 2.0}
        record = {'id': 'fixture', 'sha': 'a'*40, 'command': COMMAND,
                  'process_exit': 0, 'status': 'completed',
                  'schedule_result': {'phases': [phase], 'expected_tests': 1,
                                      'complete': True}}
        log = ('     Running tests/fixture.rs (target/debug/deps/fixture-12345678)\n'
               'running 1 test\ntest example ... ok\n'
               'test result: ok. 1 passed; 0 failed; 0 ignored; '
               '0 measured; 0 filtered out; finished in 0.01s\n')
        self.assertIn('compile | 1.0 | 64 | 0 | completed',
                      reporting.markdown(reporting.build_report(record, log)))

    def test_dag_case_set_ignores_child_repeats_but_keeps_parent_names(self):
        log = ('test parent::one ... ok\n'
               'test parent::one ... ok\n'
               'test parent::panic - should panic ... ok\n'
               'test parent::ignored ... ignored, reason\n')
        self.assertEqual(scheduler.dag_observed_cases(log),
                         {'parent::one', 'parent::panic', 'parent::ignored'})

    def test_dag_case_set_survives_concurrent_stderr_inside_result_line(self):
        log = ('restart-window test control::tests::control_capability_is_rejected_on_a_read_operation ... '
               '1ok: open\n'
               'test another::case ... ok\n'
               'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; '
               '0 filtered out; finished in 0.01s\n')
        self.assertEqual(scheduler.dag_observed_cases(log), {
            'control::tests::control_capability_is_rejected_on_a_read_operation',
            'another::case',
        })

    def test_publication_daemon_requires_feature_and_preserves_roles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / 'run'
            run.mkdir()
            source = root / 'target/debug/hirouted'
            source.parent.mkdir(parents=True)
            source.write_bytes(b'feature-qualified-daemon')
            normal = root / 'target/validation-schedule/fixture/products'
            normal.mkdir(parents=True)
            (normal / 'hiroute').write_bytes(b'cli')
            metadata = {'packages': [{'id': 'daemon-id', 'name': 'hiroute-daemon'}]}
            event = {'reason': 'compiler-artifact', 'package_id': 'daemon-id',
                     'target': {'name': 'hirouted', 'kind': ['bin']},
                     'features': ['integration-test-hooks'],
                     'executable': str(source), 'fresh': True}
            environment = {'HIROUTE_VALIDATION_PRODUCT_BIN_DIR': str(normal)}
            receipt_dir = scheduler.dag_publication_daemon(
                json.dumps(event), root, metadata, 'fixture', environment, 'a'*40, run)
            self.assertEqual((receipt_dir / 'hirouted').read_bytes(), b'feature-qualified-daemon')
            self.assertEqual((receipt_dir / 'hiroute').read_bytes(), b'cli')
            self.assertEqual(json.loads((run / 'dag-publication-receipt.json').read_text())['features'],
                             ['integration-test-hooks'])
            event['features'] = []
            with self.assertRaisesRegex(ValueError, 'feature-qualified'):
                scheduler.dag_publication_daemon(json.dumps(event), root, metadata,
                                                 'second', environment, 'a'*40, run)

    def test_dag_snapshots_only_relinked_smoke_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiled = root / 'target/debug/deps'
            compiled.mkdir(parents=True)
            smoke = compiled / 'smoke_cli-1234'
            ordinary = compiled / 'ordinary-1234'
            smoke.write_bytes(b'smoke')
            ordinary.write_bytes(b'ordinary')
            targets = [
                {'name': 'smoke_cli', 'source': str(smoke),
                 'sha256': scheduler.file_digest(smoke)},
                {'name': 'ordinary', 'source': str(ordinary),
                 'sha256': scheduler.file_digest(ordinary)},
            ]
            scheduler.dag_snapshot(targets, root, 'fixture')
            self.assertNotEqual(targets[0]['executable'], targets[0]['source'])
            self.assertEqual(Path(targets[0]['executable']).read_bytes(), b'smoke')
            self.assertEqual(targets[1]['executable'], targets[1]['source'])

    def test_dag_catalog_rejects_escaped_or_ambiguous_compiler_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / 'tools/product-e2e'
            package.mkdir(parents=True)
            source = package / 'tests/smoke_cli.rs'
            source.parent.mkdir()
            source.touch()
            executable = root / 'target/debug/deps/smoke_cli-0123456789abcdef'
            executable.parent.mkdir(parents=True)
            executable.write_text('test artifact')
            declared = {'name': 'smoke_cli', 'kind': ['test'], 'test': True,
                        'src_path': str(source)}
            metadata = {'workspace_members': ['product-package'],
                        'packages': [{'id': 'product-package', 'name': 'hiroute-product-e2e',
                                      'manifest_path': str(package / 'Cargo.toml'),
                                      'targets': [declared]}]}
            event = {'reason': 'compiler-artifact', 'package_id': 'product-package',
                     'profile': {'test': True}, 'executable': str(executable),
                     'target': {'name': 'smoke_cli', 'kind': ['test'],
                                'src_path': str(source)}}
            log = '\n'.join((json.dumps(event), json.dumps({'reason': 'build-finished',
                                                            'success': True})))
            rows = scheduler.dag_catalog(log, root, metadata)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]['label'], 'tests/smoke_cli.rs')
            missing_source = source.with_name('omitted.rs')
            missing_source.touch()
            metadata['packages'][0]['targets'].append(
                {'name': 'omitted', 'kind': ['test'], 'test': True,
                 'src_path': str(missing_source)})
            with self.assertRaisesRegex(ValueError, 'missing=1'):
                scheduler.dag_catalog(log, root, metadata)
            metadata['packages'][0]['targets'].pop()
            with self.assertRaisesRegex(ValueError, 'Malformed Cargo JSON'):
                scheduler.dag_catalog(log + '\n{truncated', root, metadata)
            with self.assertRaisesRegex(ValueError, 'build-finished=0'):
                scheduler.dag_catalog(json.dumps(event), root, metadata)
            escaped = root / 'outside'
            escaped.write_text('unexpected')
            event['executable'] = str(escaped)
            with self.assertRaisesRegex(ValueError, 'escapes'):
                scheduler.dag_catalog(json.dumps(event), root, metadata)

    def test_filters_and_feature_changes_cannot_silently_narrow_the_schedule(self):
        for suffix in (['--lib'], ['--', '--ignored'], ['--no-default-features'], ['some_filter']):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                scheduler.commands(COMMAND + suffix)
        for suffix in ([], ['--no-fail-fast']):
            for name, command in scheduler.commands(COMMAND + suffix).items():
                self.assertEqual(command[:len(COMMAND + suffix)], COMMAND + suffix)

    def test_listing_requires_unique_smoke_and_complete_names(self):
        self.assertEqual(scheduler.listed_tests('default_two_domain_smoke: test\nother: test\n2 tests, 0 benchmarks\n'), 2)
        for text in ('0 tests, 0 benchmarks\n',
                     'default_two_domain_smoke: test\n2 tests, 0 benchmarks\n',
                     'default_two_domain_smoke: test\nextra_default_two_domain_smoke: test\n2 tests, 0 benchmarks\n'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                scheduler.listed_tests(text)

    def run_fixture(self, mode='pass'):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / 'run'
            run.mkdir()
            binary = root / 'bin'
            binary.mkdir()
            test_binary = root / 'target/debug/deps/smoke_cli-deadbeef01234567'
            test_binary.parent.mkdir(parents=True)
            test_binary.write_text('#!' + sys.executable + '\n' + TEST_BINARY)
            test_binary.chmod(0o700)
            (root / 'tools/product-e2e').mkdir(parents=True)
            cargo = binary / 'cargo'
            cargo.write_text('#!' + sys.executable + '\n' + '''import os,sys,time,pathlib,json,hashlib
a=sys.argv[1:]; root=pathlib.Path.cwd(); mode=os.environ['FIXTURE_MODE']
if '--no-run' in a:
 assert os.environ['CARGO_BUILD_JOBS']=='64'
 (root/'compiled').touch()
 print('  Executable tests/smoke_cli.rs (target/debug/deps/smoke_cli-deadbeef01234567)')
 sys.exit(0)
assert (root/'compiled').exists()
assert os.environ['CARGO_BUILD_JOBS']=='8'
if '--list' in a:
 print('default_two_domain_smoke: test\\nregression: test\\nprepare_default_smoke_builds: test\\n3 tests, 0 benchmarks'); sys.exit(0)
if 'prepare_default_smoke_builds' in a:
 (root/'prepared').touch()
 n=0 if mode=='missing-preparation' else 1
 env={'CARGO_BUILD_JOBS':'8','CARGO_PKG_NAME':'hiroute-product-e2e','CARGO_BIN_EXE_hiroute-smoke':'/fixture/hiroute-smoke','RUST_RECURSION_COUNT':'1'}
 artifacts=[]
 for package in ('hiroute-e2e','hiroute-cli','hiroute-daemon'):
  binary={'hiroute-e2e':'hiroute-e2e','hiroute-cli':'hiroute','hiroute-daemon':'hirouted'}[package]
  cached=root/'target/smoke/builds'/('f'*64)/package/binary
  cached.parent.mkdir(parents=True)
  cached.write_text(package)
  sha='sha256:'+hashlib.sha256(cached.read_bytes()).hexdigest()
  artifact={'package':package,'binary':binary,'sha256':sha,'source_revision':'a'*40}
  receipt=cached.with_name('receipt.json')
  value={'artifact':artifact,'path':str(cached),'recipe':{'source_revision':'a'*40,'package':package,'binary':binary,'environment':env}}
  body=json.dumps(value)
  artifacts.append(dict(artifact,receipt={'path':str(receipt),'sha256':'sha256:'+hashlib.sha256(body.encode()).hexdigest()}))
  if mode=='missing-receipt' and package=='hiroute-cli': continue
  receipt.write_text(body)
 report=root/'target/smoke/prepare-fixture/preparation.json'
 report.parent.mkdir(parents=True)
 report.write_text(json.dumps({'schema':'hiroute.smoke.preparation/v1','source_revision':'a'*40,'scenarios_executed':0,'artifacts':artifacts}))
 print('build preparation='+str(report))
 print('Running tests/smoke_cli.rs (target/debug/deps/smoke_cli-deadbeef01234567)')
 print(f'running {n} tests')
 print(f'test result: ok. {n} passed; 0 failed; 0 ignored'); sys.exit(0)
assert (root/'prepared').exists()
assert '--skip' in a
(root/'remainder.ready').touch()
until=time.monotonic()+5
while not (root/'smoke.ready').exists():
 if time.monotonic()>until: sys.exit(98)
 time.sleep(.01)
print('Running tests/remainder.rs (target/debug/deps/remainder-abcdef0123456789)')
print('running 2 tests')
print('test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s')
passed=0 if mode=='missing-remainder' else 1
print(f'test result: ok. {passed} passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s')
''')
            cargo.chmod(0o700)
            output = io.StringIO()
            with patch.dict(os.environ, PATH=str(binary) + os.pathsep + os.environ.get('PATH', ''), FIXTURE_MODE=mode), \
                 (root / 'checkout.lock').open('a') as lease:
                result = scheduler.execute(runner, root, run, root, dict(command=COMMAND, timeout=15, sha='a'*40), lease, output)
            if mode == 'missing-preparation':
                self.assertFalse((root / 'smoke.ready').exists())
            return result, output.getvalue()

    def test_preparation_then_concurrent_lanes_cover_every_listed_test_once(self):
        result, log = self.run_fixture()
        self.assertEqual(result['process_exit'], 0, result)
        self.assertTrue(result['complete'])
        self.assertEqual(result['counts'], dict(passed=2, failed=0, ignored=1))
        self.assertEqual(log.count('test result:'), 3, 'Nested summaries are not parent test counts')
        a, b = result['phases'][-2:]
        self.assertLess(max(a['started_at'], b['started_at']), min(a['finished_at'], b['finished_at']))

    def test_missing_preparation_never_starts_runtime(self):
        result, _ = self.run_fixture('missing-preparation')
        self.assertNotEqual(result['process_exit'], 0)
        self.assertIn('exactly once', result['error'])

    def test_unattested_preparation_never_starts_runtime(self):
        result, _ = self.run_fixture('missing-receipt')
        self.assertNotEqual(result['process_exit'], 0)
        self.assertIn('receipt.json', result['error'])

    def test_red_or_omitted_tests_cannot_produce_aggregate_green(self):
        for mode in ('red-smoke', 'missing-remainder'):
            with self.subTest(mode=mode):
                result, _ = self.run_fixture(mode)
                self.assertNotEqual(result['process_exit'], 0)
                self.assertFalse(result['complete'])


class Preparation(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.sha = 'a' * 40

    def bind(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))
        return dict(path=str(path), sha256='sha256:' + scheduler.file_digest(path))

    def prepare(self, jobs):
        environment = dict(CARGO_BUILD_JOBS=str(jobs), CARGO_PKG_NAME='hiroute-product-e2e',
                           **{'CARGO_BIN_EXE_hiroute-smoke': '/fixture/hiroute-smoke',
                              'RUST_RECURSION_COUNT': '1'})
        artifacts = []
        for package, binary in (('hiroute-e2e', 'hiroute-e2e'), ('hiroute-cli', 'hiroute'),
                                ('hiroute-daemon', 'hirouted'), ('hiroute-gateway', 'hirouted')):
            key = hashlib.sha256(f'{package}-{jobs}'.encode()).hexdigest()
            path = self.root / 'target/smoke/builds' / key / package / binary
            path.parent.mkdir(parents=True)
            path.write_bytes(package.encode())  # Deliberately identical bytes across recipes.
            digest = 'sha256:' + scheduler.file_digest(path)
            if package == 'hiroute-gateway':
                attestation = dict(schema_version='hiroute.e2e.sut-build-attestation/v3',
                                   source_revision=self.sha, executable_path=str(path),
                                   executable_sha256=digest, build_nonce=str(jobs))
                receipt = dict(recipe='sha256:' + key, attestation=attestation)
                gateway = dict(schema='hiroute.e2e.current-preparation/v1', prepared=True,
                               source_revision=self.sha, executable_sha256=digest,
                               receipt=self.bind(path.with_name('receipt.json'), receipt))
            else:
                artifact = dict(package=package, binary=binary, source_revision=self.sha, sha256=digest)
                recipe = dict(package=package, binary=binary, source_revision=self.sha,
                              environment=environment)
                receipt = dict(recipe=recipe, artifact=artifact, path=str(path))
                artifacts.append(dict(artifact, receipt=self.bind(path.with_name('receipt.json'), receipt)))
        report = self.root / f'target/smoke/prepare-{jobs}/preparation.json'
        self.bind(report, dict(schema='hiroute.smoke.preparation/v1', source_revision=self.sha,
                               scenarios_executed=0, artifacts=artifacts, gateway=gateway))
        return report

    def execute(self, report):
        return scheduler.dag_products('build preparation=' + str(report) + '\n',
                                      self.root, self.sha, 'fixture')

    def test_cleanup_evidence_keeps_selected_recipes_without_scheduler_or_smoke_binaries(self):
        preparation = self.prepare(32)
        environment = self.execute(preparation)
        data = json.loads(preparation.read_text())
        receipts = [item['receipt'] for item in data['artifacts']] + [data['gateway']['receipt']]
        run = self.root / 'run'
        run.mkdir()
        (run / 'dag-targets.json').write_text('{"complete":true}')
        reporting.preserve_evidence(run, dict(checkout=str(self.root), sha=self.sha, command=COMMAND))
        manifest = json.loads((run / 'evidence.json').read_text())
        for binding in receipts:
            source = Path(binding['path'])
            key = source.relative_to(self.root).as_posix()
            self.assertIn(key, manifest['files'])
            saved = run / manifest['files'][key]['path']
            self.assertEqual(saved.read_bytes(), source.read_bytes())
            self.assertEqual('sha256:' + scheduler.file_digest(saved), binding['sha256'])
            self.assertIn('recipe', json.loads(saved.read_text()))
        self.assertTrue((run / 'artifacts/smoke/prepare-32/preparation.json').is_file())
        self.assertFalse((run / 'artifacts/validation-schedule').exists())
        self.assertTrue((Path(environment['HIROUTE_VALIDATION_PRODUCT_BIN_DIR']) / 'hiroute').is_file())
        self.assertFalse(any((run / 'artifacts/smoke/builds').glob('*/*/hiroute*')))
        self.assertEqual((run / 'dag-targets.json').read_text(), '{"complete":true}')

    def test_focused_then_dag_selects_exact_receipts_without_removing_other_recipes(self):
        focused = self.prepare(8)
        full = self.prepare(32)
        before = {path: path.read_bytes() for path in self.root.glob('target/smoke/builds/*/*/receipt.json')}
        self.assertEqual(len(before), 8)
        env8 = scheduler.prepared_environment('build preparation=' + str(focused), self.root, self.sha)
        env32 = self.execute(full)
        self.assertEqual(env8['CARGO_BUILD_JOBS'], '8')
        self.assertEqual(env32['CARGO_BUILD_JOBS'], '32')
        report = json.loads(full.read_text())
        selected = Path(report['gateway']['receipt']['path']).with_name('hirouted')
        self.assertEqual(env32['HIROUTE_VALIDATION_GATEWAY_BIN'], str(selected))
        directory = Path(env32['HIROUTE_VALIDATION_PRODUCT_BIN_DIR'])
        self.assertEqual((directory / 'hiroute').read_bytes(), b'hiroute-cli')
        self.assertEqual((directory / 'hirouted').read_bytes(), b'hiroute-daemon')
        self.assertEqual(before, {path: path.read_bytes() for path in before})

    def test_ambiguous_role_selection_is_rejected_even_with_matching_bytes(self):
        report = self.prepare(32)
        data = json.loads(report.read_text())
        data['artifacts'].append(data['artifacts'][0])
        report.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'incomplete artifacts'):
            self.execute(report)

    def test_multiple_selected_receipts_cannot_stand_in_for_one_exact_identity(self):
        report = self.prepare(32)
        data = json.loads(report.read_text())
        for item in (data['artifacts'][0], data['gateway']):
            binding = item['receipt']
            item['receipt'] = [binding, binding]
            report.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'no exact receipt binding'):
                self.execute(report)
            item['receipt'] = binding

    def test_selected_receipt_integrity_source_paths_and_environment_remain_required(self):
        # Every mutation leaves unrelated same-byte receipts available. They must
        # never rescue invalid or missing selected preparation evidence.
        mutations = ('missing-binding', 'missing-receipt', 'corrupt-receipt', 'wrong-receipt-hash',
                     'wrong-source', 'wrong-artifact-source', 'changed-binary', 'wrong-binary-hash',
                     'escaping-receipt', 'escaping-binary', 'other-role-binary',
                     'wrong-jobs', 'different-environment')
        for package in ('hiroute-cli', 'hiroute-gateway'):
            for mutation in mutations:
                if package == 'hiroute-gateway' and mutation in (
                        'wrong-artifact-source', 'wrong-jobs', 'different-environment'):
                    continue
                with self.subTest(package=package, mutation=mutation), tempfile.TemporaryDirectory() as temp:
                    self.root = Path(temp).resolve()
                    self.prepare(8)
                    report = self.prepare(32)
                    data = json.loads(report.read_text())
                    item = (data['gateway'] if package == 'hiroute-gateway'
                            else next(row for row in data['artifacts'] if row['package'] == package))
                    binding = item['receipt']
                    receipt_path = Path(binding['path'])
                    receipt = json.loads(receipt_path.read_text())
                    facts = receipt['attestation'] if package == 'hiroute-gateway' else receipt['recipe']
                    binary_key = 'executable_path' if package == 'hiroute-gateway' else 'path'
                    binary_facts = receipt['attestation'] if package == 'hiroute-gateway' else receipt
                    binary = Path(binary_facts[binary_key])
                    if mutation == 'missing-binding':
                        del item['receipt']
                    elif mutation == 'missing-receipt':
                        receipt_path.unlink()
                    elif mutation == 'corrupt-receipt':
                        receipt_path.write_text('{')
                    elif mutation == 'wrong-receipt-hash':
                        binding['sha256'] = 'sha256:' + '0' * 64
                    elif mutation == 'wrong-source':
                        facts['source_revision'] = 'b' * 40
                    elif mutation == 'wrong-artifact-source':
                        receipt['artifact']['source_revision'] = 'b' * 40
                    elif mutation == 'changed-binary':
                        binary.write_bytes(b'changed')
                    elif mutation == 'wrong-binary-hash':
                        if package == 'hiroute-gateway':
                            receipt['attestation']['executable_sha256'] = item['executable_sha256'] = 'sha256:wrong'
                        else:
                            receipt['artifact']['sha256'] = item['sha256'] = 'sha256:wrong'
                    elif mutation == 'escaping-receipt':
                        escaped = self.root / 'outside-receipt.json'
                        escaped.write_bytes(receipt_path.read_bytes())
                        receipt_path.unlink()
                        receipt_path.symlink_to(escaped)
                    elif mutation == 'escaping-binary':
                        escaped = self.root / 'outside-binary'
                        escaped.write_bytes(binary.read_bytes())
                        binary.unlink()
                        binary.symlink_to(escaped)
                    elif mutation == 'other-role-binary':
                        other = binary.parent / 'other-binary'
                        other.write_bytes(binary.read_bytes())
                        binary_facts[binary_key] = str(other)
                    elif mutation == 'wrong-jobs':
                        for row in data['artifacts']:
                            path = Path(row['receipt']['path'])
                            value = json.loads(path.read_text())
                            value['recipe']['environment']['CARGO_BUILD_JOBS'] = '8'
                            row['receipt'] = self.bind(path, value)
                        receipt = json.loads(receipt_path.read_text())
                    elif mutation == 'different-environment':
                        facts['environment']['RUST_RECURSION_COUNT'] = '2'
                    if mutation in ('wrong-source', 'wrong-artifact-source', 'wrong-binary-hash',
                                    'other-role-binary', 'different-environment'):
                        item['receipt'] = self.bind(receipt_path, receipt)
                    report.write_text(json.dumps(data))
                    with self.assertRaises((ValueError, OSError)):
                        self.execute(report)
                    self.assertFalse((self.root / 'target/validation-schedule').exists())

    def test_report_source_containment_and_unique_identity_are_required(self):
        report = self.prepare(32)
        log = 'build preparation=' + str(report) + '\n'
        for value in ('b' * 40,):
            with self.assertRaises(ValueError):
                scheduler.prepared_environment(log, self.root, value, jobs=32)
        with self.assertRaisesRegex(ValueError, 'exactly one report'):
            scheduler.prepared_environment(log + log, self.root, self.sha, jobs=32)
        escaped = self.root / 'preparation.json'
        escaped.write_bytes(report.read_bytes())
        report.unlink()
        report.symlink_to(escaped)
        with self.assertRaisesRegex(ValueError, 'outside this checkout'):
            self.execute(report)


if __name__ == '__main__':
    unittest.main()
