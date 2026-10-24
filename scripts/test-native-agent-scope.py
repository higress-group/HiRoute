#!/usr/bin/env python3
"""Offline containment/observer regressions; never execute an Agent or product."""
import ctypes
import errno
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_agent_scope', Path(__file__).with_name('native-agent-scope.py'))
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


class NativeScopeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.daily = self.base / 'daily'
        self.daily.mkdir()
        self.binary = self.daily / '.codex/packages/native/bin/client'
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text('fixture; never executed')

    def test_all_seven_get_independent_config_session_project_and_temporary_paths(self):
        inherited = {'LANG': 'en_US.UTF-8', 'HOME': str(self.daily), 'CODEX_HOME': '/daily/codex',
                     'ANTHROPIC_API_KEY': 'must-not-inherit', 'OPENAI_BASE_URL': 'must-not-inherit',
                     'HTTP_PROXY': 'must-not-inherit', 'CLAUDE_CODE_OAUTH_TOKEN': 'must-not-inherit',
                     'NODE_OPTIONS': 'must-not-inherit', 'PYTHONPATH': 'must-not-inherit',
                     'HERMES_HOME': '/daily/hermes'}
        roots = []
        with patch.object(scope.os, 'execve', side_effect=AssertionError('preparation must not execute')):
            for harness in scope.HARNESSES:
                root = self.base / harness
                prepared = scope.prepare_scope(harness, root, [str(self.binary), '--version'],
                                               daily_home=self.daily, inherited=inherited)
                env = prepared['environment']
                self.assertEqual(env['LANG'], inherited['LANG'])
                for key in ('ANTHROPIC_API_KEY', 'OPENAI_BASE_URL', 'HTTP_PROXY',
                            'CLAUDE_CODE_OAUTH_TOKEN', 'NODE_OPTIONS', 'PYTHONPATH'):
                    self.assertNotIn(key, env)
                for key in scope.PATHS:
                    self.assertTrue(Path(env[key]).is_relative_to(root))
                    self.assertTrue(Path(env[key]).is_dir())
                self.assertTrue((root / 'sessions').is_dir())
                self.assertEqual(prepared['command'][-2:], [str(self.binary), '--version'])
                self.assertEqual(root.stat().st_mode & 0o777, 0o700)
                self.assertEqual((root / 'native-isolation.sb').stat().st_mode & 0o777, 0o600)
                roots.append(env['HOME'])
        self.assertEqual(len(set(roots)), 7)

    def test_policy_keeps_exact_runtime_reads_and_denies_external_write_and_signal(self):
        root = self.base / 'owned'
        policy = scope.sandbox_policy(root, self.daily, [self.binary])
        self.assertIn('(require-not (literal ' + json.dumps(str(self.binary)) + '))', policy)
        self.assertNotIn('(require-not (subpath ' + json.dumps(str(self.daily)) + '))', policy)
        self.assertIn('(deny file-read-data (literal ', policy)
        self.assertIn('(deny signal (require-not (target same-sandbox)))', policy)
        self.assertIn('(deny file-write* (require-all (require-not (subpath ' + json.dumps(str(root)), policy)
        self.assertIn('(deny file-write* (literal ' + json.dumps(str(root / 'native-isolation.sb')), policy)
        self.assertIn('(deny file-read-xattr ', policy)

    def test_only_explicit_installed_tools_enter_the_private_path(self):
        root = self.base / 'tools'
        prepared = scope.prepare_scope('dsh', root, [str(self.binary)], expose={'node': self.binary},
                                       daily_home=self.daily, inherited={'PATH': '/ambient/bin'})
        self.assertEqual((root / 'bin/node').resolve(), self.binary)
        self.assertEqual(list((root / 'bin').iterdir()), [root / 'bin/node'])
        self.assertNotIn('/ambient/bin', prepared['environment']['PATH'])
        self.assertEqual(prepared['exposed_binaries'], {'node': str(self.binary)})
        for name in ('../node', '/node', '.', ''):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'plain name'):
                scope.prepare_scope('dsh', self.base / 'refused', [str(self.binary)],
                                    expose={name: self.binary}, daily_home=self.daily)

    def test_daily_config_session_and_broad_dependency_exceptions_are_refused(self):
        config = self.daily / '.codex/config.toml'
        config.write_text('private fixture bytes')
        session = self.daily / '.codex/sessions'
        session.mkdir()
        for dependency in (self.daily, self.daily.parent, self.daily / '.codex', config, session):
            with self.subTest(path=dependency), self.assertRaisesRegex(ValueError, 'daily native'):
                scope.prepare_scope('codex', self.base / 'refused', [str(self.binary)],
                                    read_only=[dependency], daily_home=self.daily)
        self.assertFalse((self.base / 'refused').exists())

    def test_scope_cannot_reuse_existing_root_or_daily_home(self):
        for root in (self.daily, self.daily / 'child'):
            with self.assertRaisesRegex(ValueError, 'independent'):
                scope.prepare_scope('claude', root, [str(self.binary)], daily_home=self.daily)
        root = self.base / 'owned'
        scope.prepare_scope('codex', root, [str(self.binary)], daily_home=self.daily)
        original = (root / 'native-isolation.sb').read_bytes()
        with self.assertRaises(FileExistsError):
            scope.prepare_scope('claude', root, [str(self.binary)], daily_home=self.daily)
        self.assertEqual((root / 'native-isolation.sb').read_bytes(), original)

    def test_read_dependency_symlinks_resolve_before_policy_admission(self):
        config = self.daily / '.claude.json'
        config.write_text('private fixture bytes')
        link = self.base / 'runtime-link'
        link.symlink_to(config)
        with self.assertRaisesRegex(ValueError, 'daily native'):
            scope.sandbox_policy(self.base / 'owned', self.daily, [link])

    @unittest.skipUnless(sys.platform == 'darwin' and scope.SANDBOX.is_file(), 'requires macOS containment')
    def test_actual_os_policy_keeps_owned_writes_and_denies_outside_writes_and_daily_reads(self):
        # Harmless system Python only: no Agent, product, provider or daily file read.
        outside = self.base / 'outside-fixture'
        outside.write_text('unchanged')
        root = self.base / 'os-scope'
        code = '''import ctypes,json,os,sys
from pathlib import Path
root,outside,daily=map(Path,sys.argv[1:])
lib=ctypes.CDLL('/usr/lib/libsandbox.1.dylib')
lib.sandbox_check.restype=ctypes.c_int
lib.sandbox_check.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int]
denied=lib.sandbox_check(os.getpid(),b'file-read-data',1,ctypes.c_char_p(str(daily/'.codex/auth.json').encode()))
(root/'tmp/owned').write_text('owned')
blocked=[]
for path in (outside,root/'native-isolation.sb'):
 try:path.write_text('must not change')
 except PermissionError:blocked.append(str(path))
print(json.dumps({'daily_read_denied':denied==1,'blocked_writes':blocked}))
'''
        prepared = scope.prepare_scope('codex', root, ['/usr/bin/python3', '-I', '-B', '-c', code,
                                                     str(root), str(outside), str(Path.home())])
        result = subprocess.run(prepared['command'], env=prepared['environment'], cwd=root / 'project',
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
        observed = json.loads(result.stdout)
        self.assertTrue(observed['daily_read_denied'])
        self.assertEqual(observed['blocked_writes'], [str(outside), str(root / 'native-isolation.sb')])
        self.assertEqual(outside.read_text(), 'unchanged')
        self.assertEqual((root / 'tmp/owned').read_text(), 'owned')

    @unittest.skipUnless(sys.platform == 'darwin' and scope.SANDBOX.is_file(), 'requires macOS containment')
    def test_actual_os_denies_cross_sandbox_signals_to_only_our_owned_helper(self):
        # The only signal target is this new Popen child. No PID is accepted from
        # a host inventory, daily baseline, argument or environment variable.
        helper = subprocess.Popen(['/usr/bin/python3', '-I', '-B', '-c',
                                   'import os,time;print(os.getpid(),flush=True);time.sleep(60)'],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        observer = scope.ExactProcessObserver()
        before = observer.snapshot([helper.pid])[0]
        self.signal_boundary_evidence = {'helper_before': before, 'target_source': 'new owned Popen child only',
                                         'daily_pid_signal_targets': 0, 'probe': None}
        try:
            self.assertTrue(before['exists'])
            self.assertEqual(before['ppid'], os.getpid())
            self.assertEqual(before['pgid'], helper.pid)
            self.assertTrue(select.select([helper.stdout], [], [], 5)[0], 'owned helper readiness timeout')
            self.assertEqual(int(helper.stdout.readline()), helper.pid)
            # /usr/bin/python3 is an exec trampoline on macOS. Freeze the
            # executable identity only after the helper's own readiness frame.
            before = observer.snapshot([helper.pid])[0]
            self.assertEqual(before['ppid'], os.getpid())
            self.assertEqual(before['pgid'], helper.pid)
            self.signal_boundary_evidence['helper_before'] = before
            os.kill(helper.pid, 0)  # Positive control: the trusted owner can observe its live child.
            self.signal_boundary_evidence['owner_sig0_allowed'] = True
            outside = self.base / 'outside-signal-fixture'
            outside.write_text('unchanged outside file')
            # A benign, owned read control under HOME proves actual denied open
            # without attempting to open any daily configuration/authentication.
            with tempfile.TemporaryDirectory(prefix='.hiroute-scope-read-control-', dir=Path.home()) as home_dir:
                read_control = Path(home_dir) / 'owned-read-control'
                read_control.write_text('benign owned read control')
                read_control.chmod(0o600)
                root = self.base / 'signal-scope'
                code = '''import ctypes,errno,json,os,signal,subprocess,sys
from pathlib import Path
root,outside,read_control,daily=map(Path,sys.argv[1:5]);target=int(sys.argv[5])
observed={'target_pid':target,'probe_pid':os.getpid(),'signals':[],'writes':[]}
for number in (0,signal.SIGTERM):
 try:
  os.kill(target,number)
  observed['signals'].append({'signal':number,'denied':False,'errno':None})
 except OSError as error:
  observed['signals'].append({'signal':number,'denied':isinstance(error,PermissionError),'errno':error.errno})
child_code="""import json,os,signal,sys
target=int(sys.argv[1]);rows=[]
for number in (0,signal.SIGTERM):
 try:os.kill(target,number);rows.append({'signal':number,'denied':False,'errno':None})
 except OSError as error:rows.append({'signal':number,'denied':isinstance(error,PermissionError),'errno':error.errno})
print(json.dumps(rows))
"""
child=subprocess.run(['/usr/bin/python3','-I','-B','-c',child_code,str(target)],capture_output=True,timeout=5)
observed['descendant_exit']=child.returncode
observed['descendant_signals']=json.loads(child.stdout) if child.returncode==0 else None
try:
 read_control.read_bytes()
 observed['owned_home_read_denied']=False
except PermissionError as error:
 observed['owned_home_read_denied']=True;observed['owned_home_read_errno']=error.errno
(root/'tmp/owned-write').write_text('owned write succeeds')
observed['inside_owned_write_succeeded']=(root/'tmp/owned-write').read_text()=='owned write succeeds'
for path in (outside,root/'native-isolation.sb',root/'native-scope.private.json'):
 try:
  path.write_text('must not change')
  observed['writes'].append({'path':str(path),'denied':False})
 except PermissionError as error:
  observed['writes'].append({'path':str(path),'denied':True,'errno':error.errno})
lib=ctypes.CDLL('/usr/lib/libsandbox.1.dylib');lib.sandbox_check.restype=ctypes.c_int
lib.sandbox_check.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int]
observed['daily_read_policy']=[{'path':str(daily/name),'denied':lib.sandbox_check(os.getpid(),b'file-read-data',1,ctypes.c_char_p(str(daily/name).encode()))==1} for name in ('.codex/config.toml','.codex/auth.json','.claude/settings.json','.claude/.credentials.json','.claude.json')]
print(json.dumps(observed))
'''
                prepared = scope.prepare_scope('codex', root,
                    ['/usr/bin/python3', '-I', '-B', '-c', code, str(root), str(outside),
                     str(read_control), str(Path.home()), str(helper.pid)])
                policy_before = (root / 'native-isolation.sb').read_bytes()
                manifest_before = (root / 'native-scope.private.json').read_bytes()
                result = subprocess.run(prepared['command'], env=prepared['environment'], cwd=root / 'project',
                                        capture_output=True, timeout=10)
                self.signal_boundary_evidence.update(probe_exit=result.returncode,
                    probe_stdout=result.stdout.decode(errors='replace'), probe_stderr=result.stderr.decode(errors='replace'),
                    policy_sha256=hashlib.sha256(policy_before).hexdigest())
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
                observed = json.loads(result.stdout)
                self.signal_boundary_evidence['probe'] = observed
                after = observer.snapshot([helper.pid])[0]
                self.signal_boundary_evidence.update(helper_after=after, helper_survived_identically=before == after)
                self.assertEqual(observed['signals'], [{'signal': number, 'denied': True, 'errno': errno.EPERM}
                                                      for number in (0, signal.SIGTERM)])
                self.assertEqual(observed['descendant_exit'], 0)
                self.assertEqual(observed['descendant_signals'], observed['signals'])
                self.assertIsNone(helper.poll(), 'sandbox signalled the owned outside helper')
                self.assertEqual(after, before, 'outside helper birth/path identity changed')
                self.assertTrue(observed['owned_home_read_denied'])
                self.assertTrue(observed['inside_owned_write_succeeded'])
                self.assertTrue(all(row['denied'] for row in observed['writes']))
                self.assertTrue(all(row['denied'] for row in observed['daily_read_policy']))
                self.assertEqual(outside.read_text(), 'unchanged outside file')
                self.assertEqual(read_control.read_text(), 'benign owned read control')
                self.assertEqual((root / 'native-isolation.sb').read_bytes(), policy_before)
                self.assertEqual((root / 'native-scope.private.json').read_bytes(), manifest_before)
                self.signal_boundary_evidence.update(outside_file_unchanged=True, policy_unchanged=True,
                    manifest_unchanged=True)
            self.assertFalse(read_control.exists())
            self.signal_boundary_evidence['owned_home_read_control_retired'] = True
        finally:
            if helper.poll() is None:
                # Cleanup never derives authority from an observed daily PID.
                # Keep the exact Popen child's birth/path bound before signalling.
                current = observer.snapshot([helper.pid])[0]
                self.assertEqual((current['pid'], current.get('birth'), current.get('ppid'), current.get('pgid')),
                                 (helper.pid, before['birth'], os.getpid(), helper.pid),
                                 'refuse cleanup after owned helper birth/parent/group change')
                helper.terminate()
                try:
                    helper.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.assertEqual(observer.snapshot([helper.pid])[0], before, 'refuse changed-helper kill')
                    helper.kill()
                    helper.wait(timeout=5)
            self.signal_boundary_evidence['owned_helper_cleanup_exit'] = helper.returncode
            self.signal_boundary_evidence['helper_after_cleanup'] = observer.snapshot([helper.pid])[0]
            self.assertFalse(self.signal_boundary_evidence['helper_after_cleanup']['exists'])
            helper.stdout.close()
            helper.stderr.close()


class ExactObserverTests(unittest.TestCase):
    def observer(self, reply):
        library = Mock()
        library.proc_pidinfo.side_effect = reply
        def image(pid, output, size):
            value = b'/native/client'
            ctypes.memmove(output, value + b'\0', len(value) + 1)
            return len(value)
        library.proc_pidpath.side_effect = image
        with patch.object(scope.sys, 'platform', 'darwin'), patch.object(scope.ctypes, 'CDLL', return_value=library):
            return scope.ExactProcessObserver(), library

    def test_controller_observes_only_requested_birth_identities_without_ps_or_signals(self):
        def reply(pid, flavor, argument, output, size):
            row = ctypes.cast(output, ctypes.POINTER(scope.BsdInfo)).contents
            row.pid, row.ppid, row.pgid, row.start_sec, row.start_usec = pid, 7, pid, 100, 20
            row.name = b'native-client'
            return size
        observer, library = self.observer(reply)
        rows = observer.snapshot([42, 42, 43])
        self.assertEqual(rows, [{'pid': pid, 'exists': True, 'ppid': 7, 'pgid': pid,
                                 'birth': [100, 20], 'image': '/native/client',
                                 'started': time.strftime('%a %b %e %H:%M:%S %Y', time.localtime(100))}
                                for pid in (42, 43)])
        self.assertEqual([call.args[0] for call in library.proc_pidinfo.call_args_list], [42, 43])
        self.assertEqual(set(library._mock_children), {'proc_pidinfo', 'proc_pidpath'})
        output = observer.guard_process_output([42])
        self.assertTrue(output.startswith('42 7 '))
        self.assertTrue(output.endswith(' /native/client\n'))

    def test_permission_and_abi_failures_stay_red_while_exited_processes_are_explicit(self):
        for error, size in ((errno.EPERM, 0), (0, 4)):
            def reply(*args):
                ctypes.set_errno(error)
                return size
            observer, _ = self.observer(reply)
            with self.assertRaisesRegex(RuntimeError, 'guard remains red'):
                observer.snapshot([42])
        def missing(*args):
            ctypes.set_errno(errno.ESRCH)
            return 0
        observer, _ = self.observer(missing)
        self.assertEqual(observer.snapshot([42]), [{'pid': 42, 'exists': False}])
        with self.assertRaisesRegex(RuntimeError, 'baseline process exited'):
            observer.guard_process_output([42])
        for invalid in ([True], [-1], [0], [1] * 4097):
            with self.assertRaises(ValueError):
                observer.snapshot(invalid)


if __name__ == '__main__':
    unittest.main()
