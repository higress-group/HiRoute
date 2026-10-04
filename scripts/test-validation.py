#!/usr/bin/env python3
"""Configuration and transport regressions; no Rust or product E2E claims."""
import contextlib
import importlib.util
import io
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


m = load('validation')
host = load('validation-host')


class Routing(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = self.root / 'validation.json'
        self.legacy = self.root / 'remote-rust.json'

    def read(self, platform='linux'):
        return m.configuration(self.config, platform, self.legacy)

    def write(self, targets):
        self.config.write_text(json.dumps(dict(version=1, targets=targets)))

    def test_linux_and_mac_defaults_preserve_legacy_mac_workflow(self):
        self.assertEqual(self.read()['backend'], {'transport': 'local'})
        self.assertNotIn('desktop', self.read())
        self.legacy.write_text('{"host":"linux-alias"}')
        self.assertEqual(self.read('darwin')['backend'], {'transport': 'ssh', 'host': 'linux-alias'})
        self.assertEqual(self.read('darwin')['desktop'], {'transport': 'local'})
        self.assertEqual(self.read()['backend'], {'transport': 'local'})

    def test_explicit_backend_overrides_even_stale_legacy_config(self):
        self.legacy.write_text('invalid old JSON')
        self.write({'backend': {'transport': 'local', 'jobs': 8}})
        self.assertEqual(self.read('darwin')['backend']['jobs'], 8)

    def test_bad_config_never_silently_falls_back(self):
        for target in ({'transport': 'sssh'}, {'transport': 'ssh', 'host': '-oProxyCommand=bad'},
                       {'transport': 'ssh', 'host': 'a b'}, {'transport': 'local', 'jobs': 0},
                       {'transport': 'local', 'jobs': True}, {'transport': 'local', 'paths': []}):
            self.write({'backend': target})
            with self.subTest(target=target), self.assertRaises(ValueError):
                self.read()
        self.write({'desktop': {'transport': 'ssh', 'host': 'mac'}})
        with self.assertRaisesRegex(ValueError, 'requires repo'):
            self.read()

    def test_remote_home_and_paths_are_not_expanded_on_caller(self):
        target = {'transport': 'ssh', 'host': 'mac', 'repo': '~/git/Hi Route', 'path': ['~/.cargo/bin']}
        self.write({'desktop': target})
        self.assertEqual(self.read()['desktop'], target)

    def test_candidate_defaults_to_caller_head_and_preserves_explicit_sha(self):
        with patch.object(m.subprocess, 'check_output', return_value='a' * 40 + '\n'):
            args = m.with_candidate(['run', '--ref', 'refs/heads/test', '--', 'cargo', 'check'], self.root)
        self.assertEqual(args[1:3], ['--sha', 'a' * 40])
        explicit = ['run', '--sha=' + 'b' * 40, '--', 'cargo', 'test']
        with patch.object(m.subprocess, 'check_output') as git:
            self.assertEqual(m.with_candidate(explicit, self.root), explicit)
            git.assert_not_called()

    def test_ssh_uses_json_and_keeps_user_arguments_out_of_shell(self):
        target = {'transport': 'ssh', 'host': 'mac', 'repo': '~/git/Hi Route'}
        arguments = ['status', '--session', '/tmp/a $(touch injected); "quote"/session.json']
        with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 9)) as call:
            result = m.dispatch({'desktop': target}, 'pilot', arguments)
        self.assertEqual(result, 9)
        argv = call.call_args.args[0]
        self.assertEqual(argv[0], 'ssh')
        self.assertNotIn(arguments[-1], argv[-1])
        payload = json.loads(call.call_args.kwargs['input'])
        self.assertEqual(payload['arguments'], arguments)
        self.assertEqual(payload['repo'], '~/git/Hi Route')

    def test_remote_backend_reuses_existing_queue_runner(self):
        with patch.object(m.subprocess, 'call', return_value=7) as call:
            result = m.dispatch({'backend': {'transport': 'ssh', 'host': 'linux'}}, 'backend', ['logs', 'RUN'])
        self.assertEqual(result, 7)
        self.assertEqual(call.call_args.args[0][-4:], ['--host', 'linux', 'logs', 'RUN'])

    def test_pilot_build_routes_to_desktop_with_small_tooling_bundle(self):
        target = {'transport': 'ssh', 'host': 'mac', 'repo': '~/git/HiRoute'}
        with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as call:
            m.dispatch({'desktop': target}, 'pilot-build', ['status', '--lease', 'a' * 32])
        payload = json.loads(call.call_args.kwargs['input'])
        self.assertEqual(payload['name'], 'desktop')
        self.assertEqual(set(payload['bundle']), {'pilot-builds.py', 'local-rust.py', 'remote-rust.py', 'desktop-pilot.py', 'validation-report.py'})

    def test_ssh_pilot_and_build_execute_same_caller_driver_instead_of_host_checkout(self):
        caller = self.root / 'caller'
        remote = self.root / 'remote-checkout'
        account = self.root / 'remote-home'
        for directory in (caller / 'scripts', remote / 'scripts', account):
            directory.mkdir(parents=True)
        events = self.root / 'driver-events.jsonl'
        driver = f'''import json, sys
from pathlib import Path
def record(action):
    with Path({str(events)!r}).open('a') as output:
        output.write(json.dumps(dict(origin='caller', driver=__file__, action=action, arguments=sys.argv[1:])) + '\\n')
if __name__ == '__main__':
    record(sys.argv[1])
'''
        build_driver = '''import importlib.util, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location('pilot', Path(__file__).with_name('desktop-pilot.py'))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)
pilot.record('build:' + sys.argv[1])
'''
        (caller / 'scripts/desktop-pilot.py').write_text(driver)
        (caller / 'scripts/pilot-builds.py').write_text(build_driver)
        for name in ('local-rust.py', 'remote-rust.py', 'validation-report.py'):
            (caller / 'scripts' / name).write_text('# unused fixture dependency\n')
        (caller / 'scripts/validation-host.py').write_text(Path(host.__file__).read_text())
        stale_driver = "raise SystemExit('stale host checkout driver was executed')\n"
        for name in ('desktop-pilot.py', 'pilot-builds.py'):
            (remote / 'scripts' / name).write_text(stale_driver)
        target = {'transport': 'ssh', 'host': 'fixture-mac', 'repo': str(remote)}
        real_run = subprocess.run
        transport_calls = []

        def ssh_or_child(argv, **kwargs):
            if argv[0] != 'ssh':
                return real_run(argv, **kwargs)
            payload = json.loads(kwargs['input'])
            transport_calls.append(payload)
            with patch.object(host.sys, 'platform', 'darwin'), \
                    patch.object(host.Path, 'home', return_value=account), \
                    patch.object(host, 'gui_session', return_value={'available': True}):
                return subprocess.CompletedProcess(argv, host.execute(payload))

        actions = [('pilot-build', ['status', '--lease', 'a' * 32])]
        actions += [('pilot', ['start', '--app', '/private/tmp/hiroute-desktop', '--build-run', 'b' * 32])]
        actions += [('pilot', [action, '--session', '/private/tmp/owned-session'])
                    for action in ('status', 'stop')]
        actions += [('pilot', ['config', '--frontend-dist', '/private/tmp/frontend'])]
        with patch.object(m, 'REPO', caller), patch.object(m.subprocess, 'run', side_effect=ssh_or_child):
            for action, arguments in actions:
                self.assertEqual(m.dispatch({'desktop': target}, action, arguments), 0)
        records = [json.loads(line) for line in events.read_text().splitlines()]
        self.assertEqual([record['action'] for record in records], ['build:status', 'start', 'status', 'stop', 'config'])
        self.assertEqual({record['origin'] for record in records}, {'caller'})
        self.assertEqual(len({record['driver'] for record in records}), 1)
        bundled_driver = Path(records[0]['driver'])
        self.assertTrue(bundled_driver.is_relative_to(account / '.cache/hiroute/validation-tools'))
        self.assertEqual(bundled_driver.read_text(), driver)
        self.assertEqual(records[-1]['arguments'][1:3], ['--repo', str(remote.resolve())])
        self.assertTrue(all(payload['bundle'] == transport_calls[0]['bundle'] for payload in transport_calls))
        self.assertEqual((remote / 'scripts/desktop-pilot.py').read_text(), stale_driver)

    def test_pilot_actions_without_complete_bundle_fail_without_host_driver_fallback(self):
        for action in ('pilot', 'pilot-build'):
            for bundle in (None, {'desktop-pilot.py': '# incomplete\n'}):
                p = self.payload(action, ['status'])
                p.update(name='desktop', bundle=bundle)
                with self.subTest(action=action, bundle=bundle), \
                        patch.object(host.sys, 'platform', 'darwin'), \
                        patch.object(host.subprocess, 'run') as run:
                    with self.assertRaisesRegex(ValueError, 'Incomplete Pilot tooling bundle'):
                        host.execute(p)
                    run.assert_not_called()

    def test_pilot_config_repository_comes_only_from_execution_target(self):
        for override in (['--repo', '/different/repo'], ['--repo=/different/repo']):
            p = self.payload('pilot', ['config', *override, '--frontend-dist', '/frontend'])
            p['name'] = 'desktop'
            with patch.object(host.sys, 'platform', 'darwin'), \
                    patch.object(host.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, 'Configure repo in validation.json'):
                    host.execute(p)
                run.assert_not_called()

    def test_bundle_is_content_addressed_immutable_and_private(self):
        sources = {name: '# fixture\n' for name in ('pilot-builds.py', 'local-rust.py', 'remote-rust.py', 'desktop-pilot.py', 'validation-report.py')}
        with patch.object(host.Path, 'home', return_value=self.root):
            first = host.tooling_bundle(sources)
            self.assertEqual(first, host.tooling_bundle(sources))
            self.assertEqual(first.stat().st_mode & 0o777, 0o700)
            (first / 'pilot-builds.py').write_text('changed')
            with self.assertRaisesRegex(ValueError, 'Immutable'):
                host.tooling_bundle(sources)

    def test_bundle_rejects_unknown_paths(self):
        with self.assertRaisesRegex(ValueError, 'Incomplete'):
            host.tooling_bundle({'../../outside': 'bad'})

    def test_missing_desktop_refuses_and_dry_run_never_connects(self):
        with self.assertRaisesRegex(ValueError, 'Configure a desktop'):
            m.dispatch(self.read(), 'pilot', ['status'])
        with patch.object(m.subprocess, 'run') as run, contextlib.redirect_stdout(io.StringIO()):
            m.dispatch({'desktop': {'transport': 'ssh', 'host': 'mac', 'repo': '~/git/HiRoute'}},
                       'pilot', ['status'], dry_run=True)
        run.assert_not_called()

    def payload(self, action='backend', arguments=None):
        return dict(target={'transport': 'local', 'jobs': 3}, name='backend', action=action,
                    arguments=arguments or ['run', '--ref', 'refs/heads/test', '--sha', 'a' * 40, '--', 'cargo', 'test'],
                    repo=str(self.root))

    def test_local_backend_adds_source_jobs_and_cargo_verdict(self):
        with patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 4)) as call:
            result = host.execute(self.payload())
        argv = call.call_args.args[0]
        self.assertEqual(result, 4)
        self.assertIn('--cargo-only', argv)
        self.assertEqual(argv[argv.index('--source') + 1], 'local')
        self.assertEqual(argv[argv.index('--jobs') + 1], '3')

    def test_mac_desktop_keeps_original_runner_options_and_unassessed_exit(self):
        p = self.payload('desktop')
        p.update(name='desktop', target={'transport': 'ssh'})
        with patch.object(host.sys, 'platform', 'darwin'), \
                patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)) as call:
            self.assertEqual(host.execute(p), 1)
        argv = call.call_args.args[0]
        self.assertNotIn('--source', argv)
        self.assertNotIn('--jobs', argv)
        self.assertNotIn('--cargo-only', argv)

    def test_platform_and_gui_fail_before_starting_product(self):
        p = self.payload('pilot', ['start', '--app', '/tmp/app'])
        p['name'] = 'desktop'
        with patch.object(host.sys, 'platform', 'linux'), patch.object(host.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'requires macOS'):
                host.execute(p)
            run.assert_not_called()
        with patch.object(host.sys, 'platform', 'darwin'), patch.object(host, 'gui_session', return_value={'available': False}), \
                patch.object(host.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'GUI login session unavailable'):
                host.execute(p)
            run.assert_not_called()

    def test_pilot_cli_requires_explicit_instance_socket(self):
        p = self.payload('pilot-cli', ['ping'])
        p['name'] = 'desktop'
        with patch.object(host.sys, 'platform', 'darwin'), self.assertRaisesRegex(ValueError, 'explicit'):
            host.execute(p)

    def test_frontend_argv_and_failures_propagate_without_shell(self):
        p = self.payload('frontend', ['test'])
        p['name'] = 'frontend'
        with patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 12)) as run:
            self.assertEqual(host.execute(p), 12)
        self.assertEqual(run.call_args.args[0], ['npm', '--prefix', 'apps/desktop', 'run', 'test'])

    def test_executor_runs_as_json_stdin_program(self):
        p = self.payload('frontend', ['not-a-command'])
        p['name'] = 'frontend'
        result = subprocess.run([sys.executable, str(Path(host.__file__))], input=json.dumps(p),
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn('frontend expects', result.stderr)

    def test_frontend_config_is_generated_on_execution_host_for_desktop_only(self):
        p = self.payload('desktop')
        p.update(name='desktop', target={'transport': 'ssh'}, frontend_dist='/mac/candidate/dist')
        with patch.object(host.sys, 'platform', 'darwin'), \
                patch.object(host.subprocess, 'check_output', return_value='{"build":{"frontendDist":"/mac/candidate/dist"}}') as config, \
                patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)) as run:
            host.execute(p)
        self.assertEqual(config.call_args.args[0][-2:], ['--frontend-dist', '/mac/candidate/dist'])
        self.assertIn('/mac/candidate/dist', run.call_args.kwargs['env']['TAURI_CONFIG'])
        self.assertEqual(run.call_args.kwargs['cwd'], self.root.resolve())

    def test_doctor_missing_repository_cannot_be_ready(self):
        p = self.payload('doctor', [])
        p['name'] = 'frontend'
        with patch.object(host.shutil, 'which', return_value='/usr/bin/tool'), \
                patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(host.doctor(p, self.root, host.environment(p['target'])), 2)
        self.assertFalse(json.loads(out.getvalue())['ready'])

    def test_doctor_requires_matching_pilot_version_for_pilot_readiness(self):
        scripts = self.root / 'scripts'
        scripts.mkdir()
        for name in ('local-rust.py', 'remote-rust.py', 'desktop-pilot.py'):
            (scripts / name).write_text('# fixture')
        p = self.payload('doctor', [])
        p['name'] = 'desktop'
        def run(argv, **kwargs):
            return subprocess.CompletedProcess(argv, 0, 'tauri-pilot 9.0.0' if '--version' in argv else 'a' * 40)
        with patch.object(host.shutil, 'which', return_value='/usr/bin/tool'), \
                patch.object(host.subprocess, 'run', side_effect=run), \
                patch.object(host, 'gui_session', return_value={'available': True}), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(host.doctor(p, self.root, host.environment(p['target'])), 0)
        report = json.loads(out.getvalue())
        self.assertTrue(report['ready'])
        self.assertFalse(report['pilot_prerequisites_ready'])

    def test_environment_expands_host_home_without_changing_parent(self):
        with patch.object(host.Path, 'home', return_value=self.root), patch.dict(os.environ, {'PATH': '/usr/bin'}):
            env = host.environment({'path': ['/opt/homebrew/bin']})
            self.assertEqual(os.environ['PATH'], '/usr/bin')
        self.assertTrue(env['PATH'].startswith('/opt/homebrew/bin:/usr/bin:'))

    def test_reusable_build_explicit_path_is_identical_across_login_environments(self):
        target = {'path': ['/opt/homebrew/bin', '~/.cargo/bin']}
        with patch.dict(os.environ, {'PATH': '/interactive/shell/bin'}):
            first = host.environment(target, stable_path=True)['PATH']
        with patch.dict(os.environ, {'PATH': '/ssh/shell/bin'}):
            second = host.environment(target, stable_path=True)['PATH']
        self.assertEqual(first, second)


if __name__ == '__main__':
    unittest.main()
