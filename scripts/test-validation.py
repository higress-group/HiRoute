#!/usr/bin/env python3
"""Configuration and transport regressions; no Rust or product E2E claims."""
import contextlib
import importlib.util
import io
import json
import re
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
    def test_windows_entry_routes_without_loading_unrelated_host_configuration(self):
        with patch.object(sys, 'argv', ['validation.py', 'windows', 'wait', 'a' * 32, '--timeout', '50']), \
                patch.object(m, 'configuration', side_effect=AssertionError('unrelated configuration')), \
                patch.object(m.subprocess, 'call', return_value=0) as call:
            self.assertEqual(m.main(), 0)
        self.assertEqual(call.call_args.args[0][1:],
                         [str(m.REPO / 'scripts/windows-dev.py'), 'wait', 'a' * 32, '--timeout', '50'])

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

    def test_frontend_staging_path_routes_only_to_ssh_backend_run(self):
        target = {'backend': {'transport': 'ssh', 'host': 'linux'}}
        with patch.object(m.subprocess, 'check_output', return_value='a' * 40), \
             patch.object(m.subprocess, 'call', return_value=0) as call:
            m.dispatch(target, 'backend', ['run', '--ref', 'refs/heads/example', '--', 'cargo', 'test'],
                       frontend_dist='~/.cache/candidate/dist')
        argv = call.call_args.args[0]
        self.assertEqual(argv[argv.index('run') + 1:argv.index('run') + 3],
                         ['--frontend-dist', '~/.cache/candidate/dist'])
        self.assertNotIn('--frontend-dist', argv[argv.index('--') + 1:])
        for transport, action, arguments in (('local', 'backend', ['run']),
                                              ('ssh', 'backend', ['status', 'RUN']),
                                              ('local', 'frontend', ['build'])):
            with self.subTest(transport=transport, action=action), self.assertRaises(ValueError):
                m.dispatch({action: {'transport': transport}}, action, arguments, frontend_dist='/dist')

    def test_pilot_build_routes_to_desktop_with_small_tooling_bundle(self):
        target = {'transport': 'ssh', 'host': 'mac', 'repo': '~/git/HiRoute'}
        with patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as call:
            m.dispatch({'desktop': target}, 'pilot-build', ['status', '--lease', 'a' * 32])
        payload = json.loads(call.call_args.kwargs['input'])
        self.assertEqual(payload['name'], 'desktop')
        self.assertEqual(set(payload['bundle']), {'pilot-builds.py', 'local-rust.py', 'remote-rust.py', 'desktop-pilot.py', 'pilot-native-app.py', 'validation-report.py'})

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
        for name in ('local-rust.py', 'remote-rust.py', 'pilot-native-app.py', 'validation-report.py'):
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
        sources = {name: '# fixture\n' for name in ('pilot-builds.py', 'local-rust.py', 'remote-rust.py', 'desktop-pilot.py', 'pilot-native-app.py', 'validation-report.py')}
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

    def test_real_validation_cli_forwards_wait_policy_before_cargo_boundary(self):
        scripts = self.root / 'scripts'
        scripts.mkdir()
        (scripts / 'local-rust.py').write_text('import json,sys\nprint(json.dumps(sys.argv[1:]))\n')
        self.write({'backend': {'transport': 'local', 'repo': str(self.root), 'jobs': 3}})
        for enabled in (False, True):
            arguments = ['run', '--ref', 'refs/heads/test', '--sha', 'a' * 40]
            if enabled:
                arguments.append('--wait-for-lock')
            arguments += ['--', 'cargo', 'test', '--locked']
            with self.subTest(wait=enabled):
                result = subprocess.run([sys.executable, str(Path(m.__file__)),
                                         '--config', str(self.config), 'backend', *arguments],
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr)
                forwarded = json.loads(result.stdout)
                boundary = forwarded.index('--')
                self.assertEqual(forwarded.count('--wait-for-lock'), int(enabled))
                self.assertEqual('--wait-for-lock' in forwarded[:boundary], enabled)
                self.assertEqual(forwarded[boundary + 1:], ['cargo', 'test', '--locked'])
                self.assertEqual(forwarded[forwarded.index('--source') + 1], 'local')
                self.assertEqual(forwarded[forwarded.index('--jobs') + 1], '3')
                self.assertIn('--cargo-only', forwarded[:boundary])

    def test_mac_desktop_forwards_wait_policy_without_backend_defaults(self):
        arguments = ['run', '--ref', 'refs/heads/test', '--sha', 'a' * 40,
                     '--wait-for-lock', '--', 'cargo', 'check']
        payload = self.payload('desktop', arguments)
        payload.update(name='desktop', target={'transport': 'ssh'})
        with patch.object(host.sys, 'platform', 'darwin'), \
                patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as call:
            self.assertEqual(host.execute(payload), 0)
        forwarded = call.call_args.args[0]
        self.assertEqual(forwarded.count('--wait-for-lock'), 1)
        self.assertLess(forwarded.index('--wait-for-lock'), forwarded.index('--'))
        self.assertNotIn('--cargo-only', forwarded)
        self.assertNotIn('--source', forwarded)

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

    def test_desktop_help_uses_real_runner_options_but_omits_wrapper_owned_values(self):
        repo = Path(host.__file__).resolve().parent.parent
        direct = subprocess.run([sys.executable, str(repo / 'scripts/local-rust.py'), 'run', '--help'],
                                text=True, capture_output=True, timeout=10, check=True).stdout
        for transport in ('local', 'ssh'):
            target = {'transport': transport, 'repo': str(self.root)}
            with self.subTest(transport=transport), patch.object(m.subprocess, 'run') as run, \
                    patch.object(m.subprocess, 'check_output') as git, contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(m.dispatch({'desktop': target}, 'desktop', ['run', '--help']), 0)
            run.assert_not_called()
            git.assert_not_called()
            help_text = output.getvalue()
            self.assertIn('validation.py desktop run', help_text)
            self.assertIn('exact pushed candidate from origin', help_text)
            self.assertIn('local Mac', help_text)
            self.assertIn("defaults to the caller's HEAD", help_text)
            self.assertIn('[--sha SHA]', help_text)
            for option in ('--source', '--repo', '--jobs'):
                self.assertIn(option, direct)
                self.assertNotIn(option, help_text)
            flags = lambda value: set(re.findall(r'(?m)^  (?:-h, )?(--[\w-]+)', value))
            self.assertEqual(flags(help_text), flags(direct) - {'--source', '--repo', '--jobs'})
            self.assertIn('--wait-for-lock', help_text)

    def test_real_validation_cli_desktop_help_works_before_contacting_execution_host(self):
        self.write({'desktop': {'transport': 'ssh', 'host': 'unreachable-help-fixture', 'repo': '/missing/checkout'}})
        result = subprocess.run([sys.executable, str(Path(m.__file__)), '--config', str(self.config),
                                 'desktop', 'run', '--help'], text=True, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('exact pushed candidate from origin', result.stdout)
        self.assertNotIn('--source', result.stdout)

    def test_source_overrides_explain_desktop_origin_and_local_backend_separately(self):
        for action, expected in [('desktop', 'exact pushed candidate from origin'),
                                 ('backend', 'Local backend selects committed source')]:
            for override in (['--source', 'local'], ['--source=origin']):
                p = self.payload(action, ['run', *override])
                p['name'] = action
                with self.subTest(action=action, override=override), \
                        patch.object(host.sys, 'platform', 'darwin'), patch.object(host.subprocess, 'run') as run:
                    with self.assertRaisesRegex(ValueError, expected):
                        host.execute(p)
                    run.assert_not_called()
        for override in (['--repo', '/other'], ['--jobs=7']):
            p = self.payload('desktop', ['run', *override])
            p['name'] = 'desktop'
            with patch.object(host.sys, 'platform', 'darwin'), self.assertRaisesRegex(ValueError, 'validation.json'):
                host.execute(p)

    def test_cargo_help_and_options_are_not_interpreted_as_wrapper_overrides(self):
        p = self.payload('desktop', ['run', '--ref', 'refs/heads/test', '--', 'cargo', 'check', '--help', '--jobs=3'])
        p['name'] = 'desktop'
        with patch.object(host.sys, 'platform', 'darwin'), \
                patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(host.execute(p), 0)
        argv = run.call_args.args[0]
        self.assertEqual(argv[argv.index('--') + 1:], ['cargo', 'check', '--help', '--jobs=3'])

    def test_real_parser_abbreviations_cannot_override_wrapper_source_or_configuration(self):
        spec = importlib.util.spec_from_file_location('local_rust', Path(m.__file__).with_name('local-rust.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        prefix = ['run', '--ref', 'refs/heads/test', '--sha', 'a' * 40]
        for canonical, abbreviations, value, expected in (
                ('source', ['--so', '--sou', '--sour'], 'local', 'exact pushed candidate from origin'),
                ('jobs', ['--j', '--jo', '--job'], '7', 'validation.json'),
                ('repo', ['--rep'], '/other/repository', 'validation.json')):
            for abbreviation in abbreviations:
                for override in ([abbreviation, value], [abbreviation + '=' + value]):
                    args = [*prefix, *override, '--', 'cargo', 'check']
                    # Positive control: these are accepted overrides in the real direct runner.
                    parsed = runner.argument_parser().parse_args(args)
                    self.assertEqual(str(getattr(parsed, canonical)), value)
                    p = self.payload('desktop', args)
                    p['name'] = 'desktop'
                    with self.subTest(override=override), patch.object(host.sys, 'platform', 'darwin'), \
                            patch.object(host.subprocess, 'run') as run:
                        with self.assertRaisesRegex(ValueError, expected):
                            host.execute(p)
                        run.assert_not_called()

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
        p['arguments'] = ['build']
        with patch.object(host.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
            self.assertEqual(host.execute(p), 0)
        self.assertEqual(run.call_args.args[0], ['npm', '--prefix', 'apps/desktop', 'run', 'build'])

    def test_console_frontend_build_explicitly_uses_the_provenance_owner(self):
        p = self.payload('frontend', ['build', '--console-assets'])
        p['name'] = 'frontend'
        with patch.object(host.importlib.util, 'module_from_spec') as module:
            module.return_value.build.return_value = 17
            with patch.object(host.importlib.util, 'spec_from_file_location'):
                self.assertEqual(host.execute(p), 17)
        self.assertEqual(module.return_value.build.call_args.args[0], Path(p['repo']).resolve())

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
