#!/usr/bin/env python3
"""Development-runner admission regressions, not native product evidence."""
import base64
import contextlib
import io
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch
import zipfile


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


controller, host = load('windows-dev'), load('windows-dev-host')
images = load('test-windows-native-runtime-audit')
profiles = load('windows-dev-profile')
desktop = load('windows-desktop-session')


class Runner(unittest.TestCase):
    def test_incomplete_installation_page_requires_exact_repair_and_no_storage_retry_in_both_languages(self):
        (self.root/'window.png').write_bytes(b'captured native image placeholder')
        (self.root/'ui.json').write_text('{}')
        button=dict(name='English',type='ControlType.Button',offscreen=False,runtime_id='42.1')
        zh=dict(nodes=[dict(name='安装尚未完成'),dict(name='SHELL_INSTALL_RECOVERY_REQUIRED'),
                       dict(name='相同版本及构建；缺包时寻求支持'),button])
        en=dict(nodes=[dict(name='Installation is incomplete'),dict(name='SHELL_INSTALL_RECOVERY_REQUIRED'),
                       dict(name='same version and build; obtain support if missing')])
        with patch.object(desktop,'ui_action',side_effect=[zh,en]) as action:
            self.assertEqual(desktop.failure_page_languages(self.root,Path('shell.exe'),42,'installer_recovery'),['zh','en'])
        self.assertEqual(action.call_args_list[1].args[-1],button,
                         'Native invocation needs the entire observed element for identity recheck')
        for extra in [dict(name='RECOVERY.md'),dict(name='重试',offscreen=False)]:
            with self.subTest(extra=extra),patch.object(desktop,'ui_action',return_value=dict(nodes=zh['nodes']+[extra])), \
                    self.assertRaisesRegex(ValueError,'borrowed storage advice'):
                desktop.failure_page_languages(self.root,Path('shell.exe'),42,'installer_recovery')
        with patch.object(desktop,'ui_action',return_value=dict(nodes=[n for n in zh['nodes'] if '相同版本' not in n['name']])), \
                self.assertRaisesRegex(ValueError,'exact repair/support'):
            desktop.failure_page_languages(self.root,Path('shell.exe'),42,'installer_recovery')


    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def archive(self, extra=None, mutate=None):
        image = bytearray(70)
        image[:2] = b'MZ'
        struct.pack_into('<I', image, 60, 64)
        image[64:] = b'PE\0\0\x64\x86'
        binary = self.root / 'sample.exe'
        binary.write_bytes(image)
        value = dict(source_revision='a' * 40, target='x86_64-pc-windows-msvc',
                     files={'sample.exe': dict(sha256=controller.digest(binary), bytes=len(image))})
        if mutate:
            mutate(value)
        archive = self.root / 'sample.zip'
        with zipfile.ZipFile(archive, 'w') as stream:
            stream.writestr('manifest.json', json.dumps(value))
            stream.write(binary, 'sample.exe')
            if extra:
                stream.writestr(*extra)
        return archive

    def test_stage_verifies_bytes_and_rechecks_existing_bundle(self):
        archive = self.archive()
        value = host.stage(self.root, archive, controller.digest(archive))
        self.assertEqual(value['source_revision'], 'a' * 40)
        self.assertEqual(host.stage(self.root, archive, controller.digest(archive)), value)
        (self.root / 'bundles' / value['bundle'] / 'sample.exe').write_bytes(b'changed')
        with self.assertRaises(ValueError):
            host.stage(self.root, archive, controller.digest(archive))

    def test_transfer_hash_is_required_before_extraction(self):
        archive = self.archive()
        with self.assertRaises(ValueError):
            host.stage(self.root, archive, '0' * 64)
        self.assertFalse((self.root / 'bundles').exists())

    def test_desktop_observation_admits_installed_bytes_and_rejects_changed_shell(self):
        home = self.root / 'home'
        programs = home / 'local/Programs/HiRoute'
        package = home / '.local/lib/hiroute/0.3.1-aaaaaaaaaaaa'
        data = home / '.local/share/hiroute'
        for path in (programs, package, data):
            path.mkdir(parents=True)
        files = {}
        for name in ('hiroute.exe', 'hirouted.exe'):
            (package / name).write_bytes(name.encode())
            files[name] = dict(sha256=profiles.digest(package / name))
        shell = programs / 'hiroute-shell.exe'
        shell.write_bytes(b'measured NSIS member')
        checksum = profiles.digest(shell)
        (programs / 'shell-package.json').write_text(json.dumps(dict(files={'hiroute-shell.exe': checksum})))
        (data / 'standalone.json').write_text(json.dumps(dict(install_root=str(package),
            service_definition=str(package / 'task.json'), target='x86_64-pc-windows-msvc')))
        manifest = dict(source_revision='a' * 40, target='x86_64-pc-windows-msvc', files=files, shell_sha256=checksum)
        with patch.object(desktop.Path, 'home', return_value=home), patch.dict(os.environ, LOCALAPPDATA=str(home / 'local')):
            self.assertEqual(desktop.installed(profiles, self.root, manifest)[2], shell)
            shell.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'shell differs'):
                desktop.installed(profiles, self.root, manifest)
            shell.write_bytes(b'measured NSIS member')
            (package / 'hiroute.exe').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'binary changed'):
                desktop.installed(profiles, self.root, manifest)

    def test_nsis_waits_for_exit_past_cli_watchdog_and_records_transaction_phases(self):
        locations = {name: self.root / name for name in ('data', 'shell_state')}
        for path in locations.values():
            path.mkdir()
        pending = locations['shell_state'] / 'install-pending.json'
        pending.write_text(json.dumps(dict(phase='prepared')))
        clock = [0.0]
        child = unittest.mock.Mock(pid=42)
        child.poll.side_effect = [None, None, 0]
        def advance(_):
            clock[0] += 80
            if clock[0] == 160:
                pending.unlink()
                (locations['shell_state'] / 'update-outcome.json').write_text(json.dumps(dict(status='succeeded')))
        with patch.object(desktop.subprocess, 'Popen', return_value=child), \
             patch.object(desktop.time, 'monotonic', side_effect=lambda: clock[0]), \
             patch.object(desktop.time, 'sleep', side_effect=advance):
            result = desktop.nsis_install(self.root / 'installer.exe', self.root, locations)
        self.assertEqual(result['elapsed_seconds'], 160)
        self.assertEqual(result['exit_code'], 0)
        self.assertFalse(result['timed_out'])
        self.assertEqual(result['events'][0]['changes']['shell']['phase'], 'prepared')
        self.assertEqual(result['events'][-1]['changes']['outcome']['status'], 'succeeded')
        child.kill.assert_not_called()
        self.assertEqual(json.loads((self.root / 'installer-process.json').read_text()), result)

    def test_nsis_watchdog_records_failure_and_stops_only_the_held_child(self):
        locations = {name: self.root / name for name in ('data', 'shell_state')}
        clock = [0.0]
        child = unittest.mock.Mock(pid=42)
        child.poll.return_value = None
        def advance(_):
            clock[0] += 1
        with patch.object(desktop, 'NSIS_INSTALL_TIMEOUT', 2), \
             patch.object(desktop.subprocess, 'Popen', return_value=child), \
             patch.object(desktop.time, 'monotonic', side_effect=lambda: clock[0]), \
             patch.object(desktop.time, 'sleep', side_effect=advance), \
             self.assertRaises(desktop.subprocess.TimeoutExpired):
            desktop.nsis_install(self.root / 'installer.exe', self.root, locations)
        child.kill.assert_called_once_with()
        child.wait.assert_called_once_with(timeout=30)
        result = json.loads((self.root / 'installer-process.json').read_text())
        self.assertTrue(result['timed_out'])
        self.assertNotIn('exit_code', result)


    def test_desktop_refuses_filtered_administrator_before_native_actions(self):
        with self.assertRaisesRegex(ValueError, 'filtered administrator'), patch.object(desktop.subprocess, 'run') as run:
            desktop.execute({}, self.root, self.root, {}, profiles, {'administrator_groups': True}, host.manifest, self.root)
        run.assert_not_called()


    def test_native_lock_ui_requires_current_inspection_before_stopping_any_service(self):
        with patch.object(desktop.subprocess,'run') as run,patch.object(desktop,'installed') as admit:
            with self.assertRaisesRegex(ValueError,'recent current-product inspection'):
                desktop.locked_startup_ui({},self.root,self.root,{},profiles,{},self.root)
        run.assert_not_called();admit.assert_not_called()


    def test_startup_fixture_refuses_foreign_activity_before_desktop_or_service_mutation(self):
        observed='a'*32;folder=self.root/'runs'/observed;folder.mkdir(parents=True)
        config=dict(user='owner',session_id=3)
        request=dict(product='current',observation_run=observed,config=config)
        (folder/'request.json').write_text(json.dumps(dict(operation='inspect',product='current',config=config)))
        (folder/'result.json').write_text(json.dumps(dict(passed=True,administrator=False)))
        install=self.root/'version';shell=self.root/'desktop/hiroute-shell.exe'
        facts=dict(marker=dict(record=dict(install_root=str(install))),
                   definition=dict(text=json.dumps(dict(name='HiRoute-'+'b'*64))))
        for activity in [dict(processes=[dict(ExecutablePath=str(self.root/'foreign/hirouted.exe'))]),
                         dict(tasks=[dict(TaskName='unrelated-task')])]:
            with patch.object(desktop,'installed',return_value=({},facts,shell)), \
                    patch.object(desktop,'desktop_state') as state,patch.object(desktop.subprocess,'run') as run:
                with self.assertRaisesRegex(ValueError,'unrelated product activity'):
                    desktop.startup_ui_context(request,self.root,{},profiles,activity,self.root)
            state.assert_not_called();run.assert_not_called()




    def test_desktop_helper_requires_the_exact_committed_hash(self):
        path = self.root / 'desktop.py'
        path.write_text('raise RuntimeError("must not import")')
        with self.assertRaisesRegex(ValueError, 'committed request'):
            host.desktop_helper(dict(desktop_helper_path=str(path), desktop_helper_sha256='0' * 64))

    def test_missing_windowless_runtime_refuses_before_task_or_lock_creation(self):
        request = dict(config=dict(python=str(self.root / 'python.exe')), run_id='a' * 32,
                       harness='b' * 12 + '-' + 'c' * 12, product='d' * 12 + '-' + 'e' * 12,
                       timeout=120, binary='hiroute.exe', case='native_desktop_snapshot', operation='desktop')
        with patch.object(host, 'manifest', return_value={}), \
             patch.object(host, 'pe', side_effect=FileNotFoundError('pythonw.exe')), \
             patch.object(host, 'private_runs') as prepare, self.assertRaises(FileNotFoundError):
            host.start(self.root, request)
        prepare.assert_not_called()
        self.assertFalse((self.root / 'active.json').exists())

    def test_desktop_cleanup_keeps_the_exact_windowless_action_identity(self):
        run_id = 'a' * 32
        run = self.root / 'runs' / run_id
        run.mkdir(parents=True)
        request = dict(config=dict(python=str(self.root / 'python.exe')),
                       operation='desktop', driver_path=str(self.root / 'driver.py'))
        (run / 'request.json').write_text(json.dumps(request))
        lock = self.root / 'active.json'
        lock.write_text(json.dumps(dict(run_id=run_id)))
        with patch.object(host, 'ps', side_effect=RuntimeError('active or differs')):
            with self.assertRaises(RuntimeError):
                host.finish(self.root, run_id)
        self.assertTrue(lock.exists())
        with patch.object(host, 'ps') as ps:
            self.assertTrue(host.finish(self.root, run_id)['task_removed'])
        self.assertIn(str(self.root / 'pythonw.exe'), ps.call_args.args[0])
        self.assertIn("$t.State -ne 'Ready'", ps.call_args.args[0])
        self.assertFalse(lock.exists())

    def test_nsis_uninstall_waits_on_the_staged_program_and_preserves_the_final_path_tail(self):
        command = host.nsis_uninstall_command(r'C:\Private Runs\nsis-uninstaller.exe', r'C:\Users\Standard User\AppData\Local\Programs\HiRoute')
        self.assertEqual(command, '"C:\\Private Runs\\nsis-uninstaller.exe" /S _?=C:\\Users\\Standard User\\AppData\\Local\\Programs\\HiRoute')
        for invalid in ('path\nother', 'path"other', 'path\0other'):
            with self.assertRaises(ValueError):
                host.nsis_uninstall_command('uninstaller.exe', invalid)

    def test_locked_input_desktop_refuses_before_any_native_ui_process(self):
        with patch.object(desktop, 'desktop_state', return_value={'interactive':False,'input_desktop':None}), \
             patch.object(desktop.subprocess, 'run') as run, self.assertRaisesRegex(RuntimeError, 'unlock'):
            desktop.ui_action(self.root, {'desktop_action':'snapshot'}, Path('shell.exe'), 42)
        run.assert_not_called()
        self.assertFalse(json.loads((self.root / 'desktop-state.json').read_text())['interactive'])

    def test_scroll_requires_direction_and_keeps_observed_geometry(self):
        request = dict(desktop_action='scroll', x=250, y=200)
        observed = dict(handle=42, width=755, height=600)
        with patch.object(desktop, 'desktop_state', return_value={'interactive':True}), \
             patch.object(desktop.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'explicit direction'):
                desktop.ui_action(self.root, request, Path('shell.exe'), 12, observed=observed)
            run.assert_not_called()
            request['scroll_direction'] = 'down'
            run.return_value = desktop.subprocess.CompletedProcess([], 0, b'{}', b'')
            desktop.ui_action(self.root, request, Path('shell.exe'), 12, observed=observed)
        query = json.loads((self.root / 'ui-request.json').read_text())
        self.assertEqual((query['window'],query['width'],query['height'],query['x'],query['y'],query['direction']),
                         (42,755,600,250,200,'down'))

    def test_codex_real_call_refuses_foreign_provider_profile_or_endpoint(self):
        document = dict(model_provider='hiroute', model_providers={'hiroute':{'base_url':'http://127.0.0.1:8317/v1'}},
                        profiles={'hiroute':{'model_provider':'hiroute'}})
        self.assertEqual(desktop.codex_gateway(document, None), 'http://127.0.0.1:8317/v1')
        self.assertEqual(desktop.codex_gateway(document, 'hiroute'), 'http://127.0.0.1:8317/v1')
        for changes,profile in (({'model_provider':'openai'},None), ({},'foreign'),
                ({'model_providers':{'hiroute':{'base_url':'https://example.com/v1'}}},None),
                ({'model_providers':{'hiroute':{'base_url':'http://127.0.0.1/v1'}}},None),
                ({'model_providers':{'hiroute':{'base_url':'http://user:secret@127.0.0.1:8317/v1'}}},None)):
            with self.subTest(changes=changes,profile=profile), self.assertRaises(ValueError):
                desktop.codex_gateway({**document,**changes},profile)

    def test_codex_profile_call_reads_the_standalone_native_file_and_preserves_root(self):
        original = self.root / '.codex/config.toml'
        original.parent.mkdir()
        original.write_bytes(b'model_provider = "openai"\n')
        selected = desktop.codex_config_path(self.root, 'hiroute')
        self.assertEqual(selected, original.with_name('hiroute.config.toml'))
        self.assertEqual(desktop.codex_config_path(self.root, None), original)
        document = {'model_provider':'hiroute', 'model_providers':{'hiroute':{'base_url':'http://127.0.0.1:8317/v1'}}}
        self.assertEqual(desktop.codex_gateway(document, 'hiroute'), 'http://127.0.0.1:8317/v1')
        self.assertEqual(original.read_bytes(), b'model_provider = "openai"\n')
        self.assertFalse(selected.exists())

    def test_codex_timeout_preserves_both_partial_streams_before_propagating(self):
        failure = desktop.subprocess.TimeoutExpired(['measured-codex'], 150,
            output=b'{"type":"thread.started"}\n', stderr=b'bounded native failure\n')
        with patch.object(desktop.subprocess, 'run', side_effect=failure), \
                self.assertRaises(desktop.subprocess.TimeoutExpired):
            desktop.codex_process(self.root, ['measured-codex'], {})
        self.assertEqual((self.root / 'codex.stdout').read_bytes(), failure.stdout)
        self.assertEqual((self.root / 'codex.stderr').read_bytes(), failure.stderr)

    def test_tls_diagnostics_retain_verification_failure_without_http_or_credentials(self):
        import ssl
        import socket
        with patch.object(socket, 'create_connection', side_effect=OSError('connection refused')):
            with self.assertRaisesRegex(RuntimeError, 'before any authenticated or HTTP'):
                desktop.probe_tls(self.root, dict(scheme='https', authority='example.com', port=443))
        result = json.loads((self.root / 'tls-probe.json').read_text())
        self.assertFalse(result['tls_verified'])
        self.assertFalse(result['http_request_sent'])
        self.assertFalse(result['authenticated'])
        with self.assertRaisesRegex(ValueError, 'saved HTTPS DNS target'):
            desktop.probe_tls(self.root, dict(scheme='https', authority='user:secret@example.com', port=443))

    def test_native_tls_diagnostic_uses_saved_target_and_preserves_failure(self):
        completed = desktop.subprocess.CompletedProcess([], 0,
            b'{"tls_verified":false,"authenticated":false,"http_request_sent":false,"error":"untrusted chain"}', b'')
        with patch.object(desktop.subprocess, 'run', return_value=completed), \
                self.assertRaisesRegex(RuntimeError, 'Windows system TLS verification failed'):
            desktop.probe_windows_tls(self.root, dict(scheme='https', authority='example.com', port=443))
        result = json.loads((self.root / 'tls-probe.native.json').read_text())
        self.assertFalse(result['tls_verified'])
        self.assertFalse(result['authenticated'])
        self.assertEqual(json.loads((self.root / 'tls-target.json').read_text())['authority'], 'example.com')

    def test_codex_real_call_requires_completed_turn_tool_and_exact_nonce(self):
        expected = 'HiRoute-Windows-test:ok'
        events = [{'type':'item.completed','item':{'type':'command_execution','status':'completed','exit_code':0,
                   'command':'pwsh -Command Get-Location','aggregated_output':'C:\\acceptance\\bounded-run\n'}},
                  {'type':'item.completed','item':{'type':'agent_message','text':expected}}, {'type':'turn.completed'}]
        def evidence(values):
            return desktop.codex_call_evidence(('\n'.join(json.dumps(value) for value in values)).encode(), expected, 'bounded-run')
        self.assertTrue(all(evidence(events).values()))
        for index in range(len(events)):
            self.assertFalse(all(evidence(events[:index]+events[index+1:]).values()))
        events[1]['item']['text'] = '42'
        self.assertFalse(evidence(events)['answer'])
        events[0]['item']['status'] = 'failed'
        self.assertFalse(evidence(events)['shell_tool'])
        events[0]['item'].update(status='completed',exit_code=1)
        self.assertFalse(evidence(events)['shell_tool'])

    def test_codex_real_call_refuses_environment_override_before_process_start(self):
        request = dict(config=dict(environment={},path=[]))
        with patch.dict(os.environ,OPENAI_BASE_URL='https://example.com/v1'), \
             patch.object(desktop.subprocess, 'run') as run, self.assertRaisesRegex(ValueError,'environment overrides'):
            desktop.codex_call(self.root,request,profiles)
        run.assert_not_called()

    def native_codex_inference(self, action, completed=True, answer=True):
        home = self.root / 'native-home'
        config = home / '.codex/hiroute.config.toml'
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text('model_provider = "hiroute"\n[model_providers.hiroute]\n'
                          'base_url = "http://127.0.0.1:65140/v1"\n')
        binary = self.root / 'codex.exe'
        binary.write_bytes(b'measured native client')
        run_id = 'a' * 32
        events = [{'type':'item.completed','item':{'type':'agent_message',
                   'text':'HiRoute-Windows-' + run_id + ':ok' if answer else 'unrelated answer'}}]
        if completed:
            events.append({'type':'turn.completed'})
        output = ('\n'.join(json.dumps(value) for value in events)).encode()
        request = dict(run_id=run_id, desktop_action=action, codex_profile='hiroute',
                       config=dict(environment={'HIROUTE_WORKER_CODEX_BINARY':str(binary)}, path=[]))
        parser = types.SimpleNamespace(loads=lambda text: dict(model_provider='hiroute',
            model_providers={'hiroute':{'base_url':'http://127.0.0.1:65140/v1'}}))
        version = types.SimpleNamespace(returncode=0, stdout=b'codex-cli test\n', stderr=b'')
        process = types.SimpleNamespace(returncode=0, stdout=output, stderr=b'')
        with patch.dict(os.environ, {'PATH':os.defpath}, clear=True), \
             patch.dict(sys.modules, {'tomllib':parser}), \
             patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop.subprocess, 'run', return_value=version), \
             patch.object(desktop, 'codex_process', return_value=process) as execute:
            try:
                return desktop.codex_call(self.root, request, profiles)
            finally:
                command = execute.call_args.args[1]
                self.assertEqual(command[command.index('--sandbox') + 1], 'read-only')
                self.assertIn('approval_policy="never"', command)
                self.assertEqual(command[command.index('--profile') + 1], 'hiroute')
                self.assertEqual('Do not use any tools' in command[-1], action == 'codex-inference')

    def test_pure_inference_is_distinct_from_the_unmodified_native_tool_gate(self):
        evidence = self.native_codex_inference('codex-inference')
        self.assertEqual(evidence['scope'], 'inference_only')
        self.assertTrue(evidence['turn_completed'] and evidence['answer'])
        self.assertFalse(evidence['shell_tool'])
        with self.assertRaisesRegex(RuntimeError, 'real tool and answer'):
            self.native_codex_inference('codex-call')

    def test_pure_inference_requires_both_completed_turn_and_exact_per_run_answer(self):
        for completed, answer in [(False, True), (True, False)]:
            with self.subTest(completed=completed, answer=answer), \
                 self.assertRaisesRegex(RuntimeError, 'turn and exact answer'):
                self.native_codex_inference('codex-inference', completed, answer)

    def native_codex_two_turn(self, first_thread, second_thread, second_answer=True):
        home = self.root / 'two-turn-home'
        config = home / '.codex/hiroute.config.toml'
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text('saved test profile')
        binary = self.root / 'two-turn-codex.exe'
        binary.write_bytes(b'measured native client')
        run_id = 'b' * 32
        request = dict(run_id=run_id, desktop_action='codex-two-turn', codex_profile='hiroute',
                       config=dict(environment={'HIROUTE_WORKER_CODEX_BINARY':str(binary)}, path=[]))
        parser = types.SimpleNamespace(loads=lambda text: dict(model_provider='hiroute',
            model_providers={'hiroute':{'base_url':'http://127.0.0.1:65140/v1'}}))
        def completed(thread, suffix, answer=True):
            events = ([{'type':'thread.started','thread_id':thread}] if thread else [])
            events += [{'type':'item.completed','item':{'type':'agent_message',
                       'text':'HiRoute-Windows-' + run_id + suffix if answer else 'unexpected'}},
                       {'type':'turn.completed'}]
            return desktop.subprocess.CompletedProcess([], 0,
                ('\n'.join(json.dumps(event) for event in events)).encode(), b'')
        version = desktop.subprocess.CompletedProcess([], 0, b'codex-cli test\n', b'')
        first = completed(first_thread, '-a:ok')
        second = completed(second_thread, '-b:ok', second_answer)
        with patch.dict(os.environ, {'PATH':os.defpath}, clear=True), \
             patch.dict(sys.modules, {'tomllib':parser}), \
             patch.object(desktop.Path, 'home', return_value=home), \
             patch.object(desktop.subprocess, 'run', side_effect=[version, first, second]) as execute:
            try:
                result = desktop.codex_call(self.root, request, profiles)
            except Exception as error:
                return error, execute.call_args_list
            return result, execute.call_args_list

    def test_two_turn_reuses_the_observed_native_session_and_retains_both_streams(self):
        thread = '01234567-89ab-cdef-0123-456789abcdef'
        result, calls = self.native_codex_two_turn(thread, thread)
        self.assertEqual(result['scope'], 'two_turn_inference')
        self.assertEqual(result['thread_id'], thread)
        self.assertTrue(result['first']['answer'] and result['second']['answer'])
        self.assertEqual(len(calls), 3)  # Version plus two actual client invocations.
        first, second = calls[1].args[0], calls[2].args[0]
        self.assertNotIn('resume', first)
        self.assertEqual(second[second.index('resume') + 1], thread)
        self.assertIn('previous reply was correct', second[-1])
        self.assertEqual(calls[1].kwargs['cwd'], calls[2].kwargs['cwd'])
        for name in ('codex-a.stdout', 'codex-a.stderr', 'codex-b.stdout', 'codex-b.stderr', 'codex-two-turn.json'):
            self.assertTrue((self.root / name).is_file(), name)

    def test_two_turn_refuses_unproved_or_changed_session_and_incomplete_followup(self):
        thread = '01234567-89ab-cdef-0123-456789abcdef'
        for first, second, answer, expected_calls in [
                (None, thread, True, 2),
                (thread, 'fedcba98-7654-3210-fedc-ba9876543210', True, 3),
                (thread, thread, False, 3)]:
            with self.subTest(first=first, second=second, answer=answer):
                result, calls = self.native_codex_two_turn(first, second, answer)
                self.assertIsInstance(result, (ValueError, RuntimeError))
                self.assertEqual(len(calls), expected_calls)

    def test_inspection_keeps_bounded_transaction_and_diagnostic_evidence_without_mutation(self):
        home = self.root / 'home'
        local = self.root / 'local'
        journal = home / '.local/share/hiroute/service/install-journal.json'
        journal.parent.mkdir(parents=True)
        journal.write_text('{"phase":"prepared","schema":"current"}')
        log = local / 'HiRoute/cli/diagnostics/daemon/current.jsonl'
        log.parent.mkdir(parents=True)
        log.write_bytes(b'x' * 70000 + b'\n{"code":"EXPECTED_INSTALL_FAILURE"}\n')
        original = (journal.read_bytes(), log.read_bytes())
        result = profiles.inspect_installation(home,local)
        self.assertEqual(result['install-journal.json']['record']['phase'],'prepared')
        self.assertEqual(result['diagnostics_tail']['daemon/current.jsonl']['records'],
                         [{'code':'EXPECTED_INSTALL_FAILURE'}])
        self.assertEqual(original,(journal.read_bytes(),log.read_bytes()))

    def test_installer_resume_requires_recent_matching_evidence_before_process_start(self):
        observed = self.root / 'runs' / ('a' * 32)
        observed.mkdir(parents=True)
        (observed / 'result.json').write_text(json.dumps(dict(passed=True,assessment='inspection_only')))
        (observed / 'request.json').write_text(json.dumps(dict(product='b' * 12 + '-' + 'c' * 12,
                                                            config=dict(session_id=3))))
        request = dict(observation_run='a' * 32,from_product='d' * 12 + '-' + 'e' * 12,
                       product='b' * 12 + '-' + 'c' * 12,config=dict(session_id=4))
        with patch.object(desktop.subprocess,'run') as run, self.assertRaisesRegex(ValueError,'matching installation'):
            desktop.installer_resume(request,self.root,{},profiles,{},host.manifest,self.root)
        run.assert_not_called()

        with patch.object(desktop.subprocess,'run') as run, self.assertRaisesRegex(ValueError,'explicit inspection'):
            desktop.installer_resume({**request,'observation_run':'../other'},self.root,{},profiles,{},host.manifest,self.root)
        run.assert_not_called()

    def test_installer_resume_preserves_recorded_login_and_tray_choices(self):
        facts = {'install-pending.json': {'record': {'previous_autostart': False}},
                 'shell-install.json': {'record': {'tray_enabled': False}}}
        shell = self.root / 'hiroute-shell.exe'
        completed = desktop.subprocess.CompletedProcess([], 0, b'installed', b'')
        with patch.object(desktop, 'prepared_installer_replacement', return_value=(facts, self.root, shell)), \
                patch.object(desktop.subprocess, 'run', return_value=completed) as run, \
                patch.object(profiles, 'inspect_installation', return_value={}), \
                patch.dict(os.environ, LOCALAPPDATA=str(self.root)):
            desktop.installer_resume({}, self.root, {}, profiles, {}, host.manifest, self.root)
        self.assertEqual(run.call_args.args[0], [str(shell), '--installer-install', '--no-login-start', '--no-tray'])
        self.assertEqual((self.root / 'installer-resume.stdout').read_bytes(), b'installed')

    def test_prepared_repair_accepts_only_the_measured_old_or_current_marker(self):
        home=self.root/'home';namespace=home/'.local/lib/hiroute'
        old=namespace/('0.3.1-'+'a'*12);current=namespace/('0.3.1-'+'b'*12)
        programs=home/'local/Programs/HiRoute';(programs/'standalone').mkdir(parents=True)
        prior=dict(source_revision='a'*40,files={});pm=dict(source_revision='b'*40,files={})
        for folder,record,prefix in [(old,prior,b'old'),(current,pm,b'current')]:
            folder.mkdir(parents=True)
            for name in ['hiroute.exe','hirouted.exe']:
                (folder/name).write_bytes(prefix+name.encode());record['files'][name]=dict(sha256=profiles.digest(folder/name))
                if folder==current:(programs/'standalone'/name).write_bytes((folder/name).read_bytes())
        shell=programs/'hiroute-shell.exe';shell.write_bytes(b'current measured shell');pm['shell_sha256']=profiles.digest(shell)
        pending=dict(schema='hiroute.windows-shell-transaction/v1',phase='prepared',previous_package=str(old),new={'hiroute-shell.exe':pm['shell_sha256']})
        facts={'marker':dict(sha256='marker',record=dict(install_root=str(old))),
               'install-pending.json':dict(sha256='pending',record=pending),'programs':dict(path=str(programs))}
        observation='f'*32;run=self.root/'runs'/observation;run.mkdir(parents=True)
        new_bundle='b'*12+'-'+'d'*12;old_bundle='a'*12+'-'+'c'*12
        request=dict(product=new_bundle,from_product=old_bundle,observation_run=observation,config=dict(session_id=3))
        (run/'request.json').write_text(json.dumps(dict(product=new_bundle,config=dict(session_id=3))))
        (run/'result.json').write_text(json.dumps(dict(passed=True,assessment='inspection_only',installation=facts)))
        with patch.object(desktop.Path,'home',return_value=home),patch.dict(os.environ,LOCALAPPDATA=str(home/'local')), \
                patch.object(profiles,'inspect_installation',return_value=facts):
            for accepted in [old,current]:
                facts['marker']['record']['install_root']=str(accepted)
                self.assertEqual(desktop.prepared_installer_replacement(request,pm,profiles,dict(processes=[]),lambda _:prior,self.root)[2],shell)
            foreign=namespace/('0.3.1-'+'e'*12);facts['marker']['record']['install_root']=str(foreign)
            with self.assertRaisesRegex(ValueError,'outside the admitted'):
                desktop.prepared_installer_replacement(request,pm,profiles,dict(processes=[]),lambda _:prior,self.root)
            facts['marker']['record']['install_root']=str(current);(current/'hiroute.exe').write_bytes(b'substituted current CLI')
            with self.assertRaisesRegex(ValueError,'substituted product'):
                desktop.prepared_installer_replacement(request,pm,profiles,dict(processes=[]),lambda _:prior,self.root)

    def test_nsis_retry_admits_the_exact_interrupted_product_before_normal_install(self):
        interrupted = 'b' * 12 + '-' + 'c' * 12
        request = dict(product='a' * 12 + '-' + 'd' * 12, from_product='e' * 12 + '-' + 'f' * 12,
                       recover_product=interrupted, observation_run='1' * 32)
        with patch.object(profiles, 'inspect_installation', return_value={}), \
                patch.dict(os.environ, LOCALAPPDATA=str(self.root)), \
                patch.object(desktop, 'prepared_installer_replacement') as admission, \
                patch.object(desktop, 'installed') as installed:
            desktop.admit_nsis_install(request, profiles, {}, lambda _: {'image': 'measured'}, self.root)
            self.assertEqual(admission.call_args.args[0], {**request, 'product': interrupted})
            self.assertEqual(admission.call_args.args[1], {'image': 'measured'})
            installed.assert_not_called()
            admission.side_effect = ValueError('changed interrupted image')
            with self.assertRaisesRegex(ValueError, 'changed interrupted image'):
                desktop.admit_nsis_install(request, profiles, {}, lambda _: {}, self.root)

    def test_manual_generated_file_quarantine_preserves_backup_and_refuses_changed_bytes(self):
        programs, state, output = (self.root / name for name in ('program','state','evidence'))
        programs.mkdir();(state / 'install-backups').mkdir(parents=True);output.mkdir()
        source = programs / 'uninstall.exe';source.write_bytes(b'verified generated image')
        backup = state / 'install-backups/uninstall.exe';backup.write_bytes(b'previous verified image')
        old = profiles.digest(backup);expected = profiles.digest(source)
        pending = {'old':{'uninstall.exe':old},'new':{}}
        with self.assertRaisesRegex(ValueError,'identity changed'):
            desktop.quarantine_generated_uninstaller(programs,state,output,'0'*64,pending,profiles)
        self.assertTrue(source.is_file());self.assertEqual(profiles.digest(backup),old)
        result = desktop.quarantine_generated_uninstaller(programs,state,output,expected,pending,profiles)
        self.assertEqual(result['sha256'],expected);self.assertFalse(source.exists())
        self.assertEqual(profiles.digest(output / 'unregistered-uninstaller.bin'),expected)
        self.assertEqual(profiles.digest(backup),old)

    def test_bootstrap_publishes_once_and_preserves_changed_content(self):
        destination = self.root / 'helper.py'
        temporary = self.root / 'upload'
        payload = b'validated helper'
        checksum = hashlib.sha256(payload).hexdigest()
        entries = [dict(path=str(destination),sha256=checksum)]
        def execute(publication=None):
            output=io.StringIO()
            with contextlib.redirect_stdout(output):
                exec(controller.bootstrap_code(str(self.root), entries, publication), {})
            return json.loads(output.getvalue())
        self.assertEqual(execute(), [])
        temporary.write_bytes(payload)
        self.assertEqual(execute(dict(path=str(destination),temporary=str(temporary),sha256=checksum)),[checksum])
        before=destination.stat().st_mtime_ns
        self.assertEqual(execute(), [checksum])
        self.assertEqual(destination.stat().st_mtime_ns,before)
        destination.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'identity changed'):
            execute()
        self.assertEqual(destination.read_bytes(),b'changed')

    def test_wait_bootstrap_does_not_upload_over_live_helpers(self):
        remote=controller.Remote(dict(root=str(self.root),python='python.exe',identity_file='key'))
        checksums=[controller.digest(Path(__file__).with_name(name)) for name in
                   ('windows-dev-host.py','windows-dev-profile.py','windows-desktop-session.py')]
        with patch.object(remote,'execute',return_value=json.dumps(checksums)),patch.object(remote,'upload') as upload:
            remote.bootstrap()
        upload.assert_not_called()

    def test_archive_traversal_and_unmanifested_entries_fail(self):
        for name in ('../escape', '/absolute', 'C:/absolute', 'dir\\file.exe', 'other.exe'):
            with self.subTest(name=name):
                archive = self.archive(extra=(name, b'bad'))
                with self.assertRaises(ValueError):
                    host.stage(self.root, archive, controller.digest(archive))
        self.assertFalse((self.root / 'bundles').exists())

    def test_failed_stage_is_not_reused(self):
        archive = self.archive(mutate=lambda v: v['files']['sample.exe'].update(sha256='0' * 64))
        with self.assertRaises(ValueError):
            host.stage(self.root, archive, controller.digest(archive))
        with self.assertRaisesRegex(ValueError, 'Previous stage failed'):
            host.stage(self.root, archive, controller.digest(archive))

    def test_wrong_architecture_is_rejected(self):
        archive = self.archive()
        binary = self.root / 'sample.exe'
        content = bytearray(binary.read_bytes())
        content[-2:] = b'\x64\xaa'
        binary.write_bytes(content)
        with self.assertRaises(ValueError):
            host.pe(binary)

    def test_nsis_stub_is_admitted_only_as_the_declared_installer(self):
        payload = bytearray(70)
        payload[:2] = b'MZ'
        struct.pack_into('<I', payload, 60, 64)
        payload[64:] = b'PE\0\0\x4c\x01'
        entry = dict(sha256=hashlib.sha256(payload).hexdigest(), bytes=len(payload))
        for declaration in (None, 'nsis'):
            def mutate(value):
                value['files']['installer.exe'] = entry
                if declaration:
                    value['installer_format'] = declaration
            archive = self.archive(extra=('installer.exe', payload), mutate=mutate)
            if declaration:
                staged = host.stage(self.root, archive, controller.digest(archive))
                self.assertIn('installer.exe', staged['files'])
                path = self.root / 'bundles' / staged['bundle'] / 'installer.exe'
                with self.assertRaises(ValueError):
                    host.pe(path)  # Ordinary product/harness admission stays x64.
            else:
                with self.assertRaisesRegex(ValueError, 'explicit NSIS'):
                    host.stage(self.root, archive, controller.digest(archive))

    def test_desktop_bundle_binds_installer_and_compiled_harness_to_the_candidate(self):
        sha = 'a' * 40
        binaries = self.root / 'target/x86_64-pc-windows-msvc/release'
        binaries.mkdir(parents=True)
        harness = binaries / 'windows_desktop.exe'
        harness.write_bytes(images.image())
        payload = self.root / 'apps/shell/src-tauri/resources/standalone'
        payload.mkdir(parents=True)
        for name in ('hiroute.exe', 'hirouted.exe'):
            (payload / name).write_bytes(b'embedded-' + name.encode())
        (payload / 'package-manifest.json').write_text(json.dumps(dict(revision=sha,
            target='x86_64-pc-windows-msvc', files={
                'bin/' + name: dict(sha256=controller.digest(payload / name), size=(payload / name).stat().st_size)
                for name in ('hiroute.exe', 'hirouted.exe')})))
        assets = self.root / 'target/candidate/assets'
        assets.mkdir(parents=True)
        installer = assets / 'candidate.exe'
        installer.write_bytes(b'component-nsis-fixture')
        result = dict(revision=sha, target='x86_64-pc-windows-msvc', status='awaiting_runtime',
                      package_integrity='green', assets=str(assets),
                      desktop_result=dict(revision=sha, native_runtime='not_executed',
                                          shell_image=dict(sha256='c' * 64),
                                          installer_sha256=controller.digest(installer), installer_size=installer.stat().st_size,
                                          standalone_manifest_sha256=controller.digest(payload / 'package-manifest.json')),
                      website_artifact=dict(kind='desktop', format='nsis', platform='Windows', architecture='x86_64',
                                            filename=installer.name, sha256=controller.digest(installer)))
        record = self.root / 'result.json'
        record.write_text(json.dumps(result))
        log = self.root / 'compiler.jsonl'
        log.write_text(json.dumps(dict(reason='compiler-artifact', target=dict(name='windows_desktop'),
                                       profile=dict(test=True), executable=str(harness))) + '\n')
        with patch.object(controller, 'REPO', self.root), \
             patch.object(controller.subprocess, 'check_output', return_value=sha):
            output = self.root / 'desktop.zip'
            exported = controller.bundle_desktop(record, log, output)
            self.assertEqual(exported['native_acceptance'], 'not_executed')
            with zipfile.ZipFile(output) as archive:
                manifest = json.loads(archive.read('manifest.json'))
                self.assertEqual(manifest['installer_format'], 'nsis')
                self.assertEqual(manifest['shell_sha256'], 'c' * 64)
                self.assertEqual(archive.read('hirouted.exe'), b'embedded-hirouted.exe')
            harness.write_bytes(images.image(('KERNEL32.dll', 'VCRUNTIME140.dll')))
            with self.assertRaisesRegex(ValueError, 'static CRT'):
                controller.bundle_desktop(record, log, self.root / 'dynamic-harness.zip')
            harness.write_bytes(images.image())
            installer.write_bytes(b'changed installer')
            with self.assertRaisesRegex(ValueError, 'measured build identity'):
                controller.bundle_desktop(record, log, self.root / 'changed.zip')
            self.assertFalse((self.root / 'changed.zip').exists())
            installer.write_bytes(b'component-nsis-fixture')
            (payload / 'hirouted.exe').write_bytes(b'changed daemon')
            with self.assertRaisesRegex(ValueError, 'Embedded product bytes'):
                controller.bundle_desktop(record, log, self.root / 'changed-product.zip')

    def test_installation_inspection_reads_owned_paths_without_starting_or_changing_them(self):
        native_home, local = self.root / 'account', self.root / 'local'
        data = native_home / '.local/share/hiroute'
        data.mkdir(parents=True)
        install = native_home / '.local/lib/hiroute/version'
        install.mkdir(parents=True)
        (install / 'hiroute.exe').write_bytes(b'existing-cli')
        definition = install / 'task.json'
        definition.write_text('{"manager":"task"}')
        marker = data / 'standalone.json'
        marker.write_text(json.dumps(dict(install_root=str(install), service_definition=str(definition))))
        facts = profiles.inspect_installation(native_home, local)
        self.assertTrue(facts['data']['exists'])
        self.assertFalse(facts['state']['exists'])
        self.assertEqual(facts['installed_cli']['sha256'], controller.digest(install / 'hiroute.exe'))
        self.assertEqual(facts['definition']['text'], '{"manager":"task"}')
        self.assertTrue(marker.exists())
        self.assertFalse(local.exists())
        foreign = self.root / 'foreign'
        foreign.mkdir()
        (local / 'Programs').mkdir(parents=True)
        (local / 'Programs/HiRoute').symlink_to(foreign, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'Reparse'):
            profiles.inspect_installation(native_home, local)

    def test_symlink_and_multiple_links_are_not_runner_storage(self):
        archive = self.archive()
        value = host.stage(self.root, archive, controller.digest(archive))
        directory = self.root / 'bundles' / value['bundle']
        os.link(directory / 'sample.exe', self.root / 'second.exe')
        with self.assertRaises(ValueError):
            host.manifest(directory)
        (self.root / 'linked').symlink_to(directory, target_is_directory=True)
        with self.assertRaises(ValueError):
            host.plain(self.root / 'linked/sample.exe')

    def orphaned_profile(self):
        native_home, local = self.root / 'native', self.root / 'appdata'
        roots = profiles.locations(native_home, local)
        for name in ('data', 'state', 'shell_state'):
            roots[name].mkdir(parents=True)
            (roots[name] / 'retained.txt').write_bytes(name.encode())
        evidence = self.root / 'private-evidence'
        evidence.mkdir()
        return native_home, local, roots, evidence

    def test_candidate_uninstall_requires_measured_images_and_rejects_other_active_work(self):
        native_home, local, roots, _ = self.orphaned_profile()
        sha = 'a' * 40
        installed = native_home / '.local/lib/hiroute' / ('0.3.1-' + sha[:12])
        installed.mkdir(parents=True)
        files = {}
        for name in ('hiroute.exe', 'hirouted.exe'):
            (installed / name).write_bytes(name.encode())
            files[name] = dict(sha256=controller.digest(installed / name))
        definition = roots['data'] / 'task.json'
        definition.write_text(json.dumps(dict(manager='task', name='HiRoute-candidate')))
        marker = dict(schema_version='hiroute.standalone-install/v1', target='x86_64-pc-windows-msvc',
                      install_root=str(installed), service_definition=str(definition))
        (roots['data'] / 'standalone.json').write_text(json.dumps(marker))
        roots['programs'].mkdir(parents=True)
        for name in ('hiroute-shell.exe', 'uninstall.exe'):
            (roots['programs'] / name).write_bytes(name.encode())
        inventory = {n: controller.digest(roots['programs'] / n) for n in ('hiroute-shell.exe', 'uninstall.exe')}
        (roots['programs'] / 'shell-package.json').write_text(json.dumps(dict(files=inventory)))
        inspected = profiles.inspect_installation(native_home, local)
        product = dict(source_revision=sha, target=marker['target'], files=files)
        activity = dict(administrator_groups=False, processes=[], tasks=[])
        result = profiles.candidate_uninstaller(native_home, local, inspected, product,
                                               inventory['hiroute-shell.exe'], activity)
        self.assertEqual(result, (roots['programs'] / 'uninstall.exe', installed))
        with self.assertRaisesRegex(ValueError, 'measured inventory'):
            profiles.candidate_uninstaller(native_home, local, inspected, product, 'f' * 64, activity)
        activity['processes'] = [dict(ExecutablePath=str(roots['programs'] / 'hiroute-shell.exe'))]
        with self.assertRaisesRegex(ValueError, 'Another product or installer'):
            profiles.candidate_uninstaller(native_home, local, inspected, product,
                                           inventory['hiroute-shell.exe'], activity)
        self.assertTrue((installed / 'hirouted.exe').exists())

    def test_archive_long_paths_use_extended_drive_and_unc_forms(self):
        self.assertEqual(profiles.extended_windows_path(r'C:\account\data'), r'\\?\C:\account\data')
        self.assertEqual(profiles.extended_windows_path(r'\\server\share\data'), r'\\?\UNC\server\share\data')
        self.assertEqual(profiles.extended_windows_path(r'\\?\C:\data'), r'\\?\C:\data')

    def test_fresh_nsis_case_rejects_old_payloads_before_installation(self):
        native_home = self.root / 'account'
        profiles.require_fresh_nsis_packages(native_home)
        packages = native_home / '.local/lib/hiroute'
        packages.mkdir(parents=True)
        profiles.require_fresh_nsis_packages(native_home)
        old = packages / 'old-version'
        old.mkdir()
        with self.assertRaisesRegex(ValueError, 'retained package roots'):
            profiles.require_fresh_nsis_packages(native_home)
        self.assertTrue(old.exists())

    def test_orphaned_data_is_archived_with_complete_hashes_and_native_files_untouched(self):
        native_home, local, roots, evidence = self.orphaned_profile()
        native = native_home / '.codex/config.toml'
        native.parent.mkdir()
        native.write_text('model="unrelated"')
        inspected = profiles.inspect_installation(native_home, local)
        result = profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                    dict(administrator_groups=False, processes=[], tasks=[]))
        self.assertEqual(result['assessment'], 'preparation_only')
        for name in ('data', 'state', 'shell_state'):
            self.assertFalse(roots[name].exists())
            self.assertEqual((evidence / ('previous-' + name) / 'retained.txt').read_bytes(), name.encode())
        manifest = json.loads((evidence / 'profile-backup.json').read_text())
        self.assertEqual(manifest['paths'][0]['inventory']['retained.txt']['sha256'],
                         hashlib.sha256(b'data').hexdigest())
        self.assertEqual(native.read_text(), 'model="unrelated"')

    def test_orphaned_partial_shell_backup_is_preserved_with_its_hash(self):
        native_home, local, roots, evidence = self.orphaned_profile()
        backup = roots['shell_state'] / 'install-backups/hiroute-shell.exe'
        backup.parent.mkdir()
        backup.write_bytes(b"retained previous shell")
        inspected = profiles.inspect_installation(native_home, local)
        profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                                      dict(administrator_groups=False))
        self.assertEqual((evidence / 'previous-shell_state/install-backups/hiroute-shell.exe').read_bytes(),
                         b"retained previous shell")
        manifest = json.loads((evidence / 'profile-backup.json').read_text())
        shell = next(p for p in manifest['paths'] if p['source'] == str(roots['shell_state']))
        self.assertEqual(shell['inventory'][str(Path('install-backups/hiroute-shell.exe'))]['sha256'],
                         hashlib.sha256(b"retained previous shell").hexdigest())
        self.assertFalse(roots['shell_state'].exists())

    def test_live_installation_activity_and_agent_references_prevent_archival(self):
        native_home, local, roots, evidence = self.orphaned_profile()
        inspected = profiles.inspect_installation(native_home, local)
        for activity in [dict(administrator_groups=True),
                         dict(administrator_groups=False, processes=[123]),
                         dict(administrator_groups=False, tasks=['HiRoute-task'])]:
            with self.assertRaises(ValueError):
                profiles.archive_orphaned_data(native_home, local, evidence, inspected, activity)
            self.assertTrue(roots['state'].exists())
        native = native_home / '.codex/config.toml'
        native.parent.mkdir()
        native.write_text(str(roots['state']))
        with self.assertRaisesRegex(ValueError, 'Native Agent configuration'):
            profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                                          dict(administrator_groups=False))
        self.assertFalse((evidence / 'profile-backup.json').exists())

    def test_changed_or_linked_orphaned_data_is_preserved_before_any_move(self):
        native_home, local, roots, evidence = self.orphaned_profile()
        inspected = profiles.inspect_installation(native_home, local)
        (roots['state'] / 'new.txt').touch()
        with self.assertRaisesRegex(ValueError, 'changed since inspection'):
            profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                                          dict(administrator_groups=False))
        (roots['state'] / 'dangling').symlink_to(self.root / 'missing')
        inspected = profiles.inspect_installation(native_home, local)
        with self.assertRaisesRegex(ValueError, 'Reparse'):
            profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                                          dict(administrator_groups=False))
        self.assertTrue(roots['data'].exists())
        self.assertFalse((evidence / 'previous-data').exists())

    def test_failed_second_archive_move_restores_the_first_directory(self):
        native_home, local, roots, evidence = self.orphaned_profile()
        inspected = profiles.inspect_installation(native_home, local)
        rename = Path.rename
        def fail_second(source, target):
            if source == roots['state']:
                raise PermissionError('fixture file lock')
            return rename(source, target)
        with patch.object(Path, 'rename', fail_second), self.assertRaises(PermissionError):
            profiles.archive_orphaned_data(native_home, local, evidence, inspected,
                                          dict(administrator_groups=False))
        self.assertEqual((roots['data'] / 'retained.txt').read_bytes(), b'data')
        self.assertEqual((roots['state'] / 'retained.txt').read_bytes(), b'state')
        self.assertFalse((evidence / 'previous-data').exists())
        outcome = json.loads((evidence / 'profile-backup-outcome.json').read_text())
        self.assertEqual(outcome['outcome'], 'restored_after_error')

    def test_config_contains_paths_not_tokens_and_lives_in_home(self):
        identity = self.root / 'key'
        identity.touch()
        value = dict(version=1, host='win-alias', identity_file=str(identity),
                     python='C:/Python/python.exe', root='C:/Validation', user='acceptance',
                     session_id=2, environment={'HIROUTE_WORKER_NODE': 'C:/Node/node.exe'}, path=[])
        config = self.root / 'config.json'
        config.write_text(json.dumps(value))
        config.chmod(0o600)
        with patch.object(controller.Path, 'home', return_value=self.root):
            self.assertEqual(controller.configuration(config)['host'], 'win-alias')
            value['environment']['API_TOKEN'] = 'forbidden'
            config.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                controller.configuration(config)
        with patch.object(controller.Path, 'home', return_value=self.root / 'other'):
            with self.assertRaises(ValueError):
                controller.configuration(config)

    def test_identifiers_and_paths_cannot_change_task_or_bundle_scope(self):
        for value in ('../foreign', 'a;whoami', 'a' * 31, '-bad'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                host.task_name(value)
        for value in ('relative', 'C:/root/../foreign', 'C:/root\nforeign'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                controller.windows_path(value)

    def test_temporary_acl_cases_preserve_an_existing_product_profile(self):
        home, local = self.root / 'home', self.root / 'local'
        marker = home / '.local/share/hiroute/standalone.json'
        marker.parent.mkdir(parents=True)
        marker.write_bytes(b'preserve installed product')
        cases = (
            'windows_artifact_restart_restores_original_content_and_native_acl',
            'windows_acl_change_loses_compensation_ownership_even_with_equal_content',
            'windows_backup_authentication_binds_native_permission_facts',
            'windows_owned_directory_cleanup_preserves_user_additions_and_replacements',
        )
        for case in cases:
            request = dict(binary='hiroute_local_storage.exe', ignored=False,
                           case='control::windows_artifacts_tests::' + case)
            with self.subTest(case=case):
                self.assertEqual(host.admit_test_profile(request, home, local), 'temporary_artifact_component')
                for changed in ({'binary': 'windows_headless.exe'}, {'ignored': True},
                                {'case': request['case'] + '_unknown'}):
                    with self.assertRaisesRegex(ValueError, 'existing installation'):
                        host.admit_test_profile({**request, **changed}, home, local)
        self.assertEqual(marker.read_bytes(), b'preserve installed product')
        marker.unlink()
        self.assertEqual(host.admit_test_profile(dict(binary='windows_headless.exe',
            ignored=True, case='actual_windows_complete_public_headless_business'), home, local),
            'disposable_installation_profile')

    def test_shell_payload_preserves_quotes_without_interpolation(self):
        value = "C:/space ' $env:PATH ; directory/file.py"
        script = '& ' + controller.ps_literal(value)
        command = controller.powershell(script)
        self.assertEqual(base64.b64decode(command.split()[-1]).decode('utf-16le'), script)
        self.assertEqual(controller.ps_literal(value), "'C:/space '' $env:PATH ; directory/file.py'")

    def test_long_remote_source_uses_stdin_before_windows_shell_command_limit(self):
        remote = controller.Remote(dict(host='windows.example.test', identity_file='key'))
        for script in ("Write-Output 'short'", "# 中文 ' $env:PATH ;\n" * 2000):
            reply = controller.subprocess.CompletedProcess([], 0, stdout='observed', stderr='')
            with patch.object(controller.subprocess, 'run', return_value=reply) as run:
                self.assertEqual(remote.execute(script), 'observed')
            command = run.call_args.args[0][-1]
            self.assertLess(len(command), 6000)
            payload = run.call_args.kwargs.get('input')
            if len(controller.powershell(script)) > 6000:
                self.assertEqual(base64.b64decode(payload).decode('utf-8'), script)
                bridge = base64.b64decode(command.split()[-1]).decode('utf-16le')
                self.assertIn('[Console]::In.ReadLine()', bridge)
                self.assertNotIn('ReadToEnd', bridge)
                self.assertTrue(payload.endswith('\n'))
                self.assertIn('$line.Length -ne ' + str(len(payload) - 1), bridge)
            else:
                self.assertIsNone(payload)
                self.assertEqual(base64.b64decode(command.split()[-1]).decode('utf-16le'), script)

    def test_long_remote_submission_failure_is_not_replayed(self):
        remote = controller.Remote(dict(host='windows.example.test', identity_file='key'))
        reply = controller.subprocess.CompletedProcess([], 1, stdout='', stderr='unknown outcome')
        with patch.object(controller.subprocess, 'run', return_value=reply) as run:
            with self.assertRaisesRegex(RuntimeError, 'no automatic replay'):
                remote.execute('# source\n' * 2000)
        self.assertEqual(run.call_count, 1)

    def test_unknown_task_outcome_retains_owner_lock(self):
        archive = self.archive()
        value = host.stage(self.root, archive, controller.digest(archive))
        request = dict(run_id='a' * 32, config=dict(python='C:/python.exe', user='acceptance'),
                       harness=value['bundle'], product=value['bundle'], binary='sample.exe',
                       case='selected_test', timeout=30, driver_path='C:/runner.py')
        with patch.object(host, 'private_runs'), patch.object(host, 'ps', side_effect=RuntimeError('unknown submission')):
            with self.assertRaises(RuntimeError):
                host.start(self.root, request)
            with self.assertRaises(FileExistsError):
                host.start(self.root, dict(request, run_id='b' * 32))
        self.assertEqual(json.loads((self.root / 'active.json').read_text())['run_id'], 'a' * 32)

    def test_recovery_refuses_active_or_foreign_configuration(self):
        run_id = 'a' * 32
        run = self.root / 'runs' / run_id
        run.mkdir(parents=True)
        request = dict(case='actual_windows_complete_public_headless_business',
                       config=dict(user='acceptance'))
        (run / 'request.json').write_text(json.dumps(request))
        home = self.root / 'home'
        (home / '.claude').mkdir(parents=True)
        settings = home / '.claude/settings.json'
        settings.write_text('{"daily_setting":true}')
        (run / 'result.json').write_text('{"passed":false}')
        with patch.object(host, 'task_state', return_value=dict(state='absent')), \
             patch.object(host.Path, 'home', return_value=home), \
             patch.dict(os.environ, USERNAME='acceptance', LOCALAPPDATA=str(home / 'local')):
            with self.assertRaisesRegex(ValueError, 'untouched disposable marker'):
                host.recover(self.root, run_id)
            self.assertTrue(settings.exists())
            (self.root / 'active.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'Finish the stopped run'):
                host.recover(self.root, run_id)
            (self.root / 'active.json').unlink()
            settings.write_text('{"windows_headless_preserved":true}')
            (run / 'result.json').touch()
            value = host.recover(self.root, run_id)
            self.assertEqual(value['preserved'], ['recovered-claude-settings.json'])
            self.assertFalse(settings.exists())

    def test_task_success_without_case_result_is_failure(self):
        (self.root / 'runs' / ('a' * 32)).mkdir(parents=True)
        with patch.object(host, 'task_state', return_value=dict(state='Ready', result=0)):
            self.assertEqual(host.status(self.root, 'a' * 32)['state'], 'failed')

    def test_interrupted_gateway_recovery_preserves_only_its_verified_fixture(self):
        home = self.root / 'home'
        data = home / '.local/share/hiroute'
        install = data / 'Windows headless 测试 123'
        stable = home / '.local/bin'
        state = home / 'local/HiRoute/cli'
        run = self.root / 'runs' / ('a' * 32)
        for path in (install, stable, state, run):
            path.mkdir(parents=True)
        request = dict(run_id='a' * 32, harness='b' * 12 + '-' + 'c' * 12,
                       product='d' * 12 + '-' + 'e' * 12, binary='windows_headless.exe',
                       config=dict(user='acceptance', session_id=2),
                       case='windows_gateway_foreground_recovery::actual_windows_foreground_gateway_recovery')
        (run / 'request.json').write_text(json.dumps(request))
        (run / 'result.json').write_text('{"passed":false,"error_type":"TimeoutError"}')
        marker = data / 'standalone.json'
        marker.write_text(json.dumps(dict(schema_version='hiroute.standalone-install/v1',
                         target='x86_64-pc-windows-msvc', install_root=str(install),
                         service_definition=str(install / 'fixture-task.xml'))))
        os.utime(run / 'request.json', (time.time() - 5,) * 2)
        files = {}
        for name in ('hiroute.exe', 'hirouted.exe'):
            image = install / name
            image.write_bytes(name.encode())
            os.link(image, stable / name)
            files[name] = dict(sha256=host.digest(image))
        (run / 'result.json').touch()
        with patch.object(host, 'task_state', return_value=dict(state='absent')), \
             patch.object(host.Path, 'home', return_value=home), \
             patch.dict(os.environ, USERNAME='acceptance', LOCALAPPDATA=str(home / 'local')), \
             patch.object(host, 'manifest', return_value=dict(files=files)), \
             patch.object(host, 'ps', side_effect=['[]', '1', '[456]', '0']), \
             patch.object(host, 'stop_interrupted_daemon') as stop:
            with self.assertRaisesRegex(ValueError, 'still has live processes'):
                host.recover(self.root, 'a' * 32)
            self.assertTrue(marker.exists())
            self.assertTrue((stable / 'hirouted.exe').exists())
            value = host.recover(self.root, 'a' * 32)
            stop.assert_called_once_with(456, install / 'hirouted.exe', request['config'],
                                        (run / 'request.json').stat().st_mtime,
                                        (run / 'result.json').stat().st_mtime)
        self.assertFalse(marker.exists())
        self.assertFalse((stable / 'hirouted.exe').exists())
        self.assertEqual((run / 'interrupted-installation/hirouted.exe').read_bytes(), b'hirouted.exe')
        self.assertEqual(len(value['preserved']), 3)

    def test_evidence_download_uses_sftp_path_syntax(self):
        self.assertEqual(controller.remote_evidence_path(
            r'C:\Validation\runs\sample', 'result.json'),
            'C:/Validation/runs/sample/result.json')

    def test_active_runtime_stop_requires_selected_panic_and_owner(self):
        run_id = 'a' * 32
        run = self.root / 'runs' / run_id
        run.mkdir(parents=True)
        request = dict(binary='windows_headless.exe', config=dict(user='acceptance'),
                       case='actual_windows_public_headless_business_without_claude')
        (run / 'request.json').write_text(json.dumps(request))
        (self.root / 'active.json').write_text(json.dumps(dict(run_id=run_id)))
        (run / 'stderr.log').write_text('ordinary progress')
        with patch.object(host, 'task_state', return_value=dict(state='Running')), \
             patch.dict(os.environ, USERNAME='acceptance'), \
             patch.object(host, 'recover_interrupted_fixture') as recover:
            with self.assertRaisesRegex(ValueError, 'has not reported a panic'):
                host.stop_panicking_runtime(self.root, run_id)
            recover.assert_not_called()
            (run / 'stderr.log').write_text("thread '" + request['case'] + "' (123) panicked at fixture.rs:1:1:\n")
            host.stop_panicking_runtime(self.root, run_id)
            recover.assert_called_once_with(self.root, run, request, stop_only=True)
            (run / 'result.json').write_text('{"passed":false}')
            with self.assertRaisesRegex(ValueError, 'active nonterminal run'):
                host.stop_panicking_runtime(self.root, run_id)


if __name__ == '__main__':
    unittest.main()
