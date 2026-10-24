#!/usr/bin/env python3
"""Process lifecycle regressions. No Agent, model, service or daily config."""
import contextlib
import importlib.util
import json
import os
import select
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name('native-agent-supervisor.py')
spec = importlib.util.spec_from_file_location('native_supervisor', SOURCE)
owner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner)


@contextlib.contextmanager
def direct_anchor(command, evidence_root):
    """Control the real private pipes without timing a guardian process kill."""
    control_r, control_w = os.pipe()
    event_r, event_w = os.pipe()
    native_stdin = os.open(os.devnull, os.O_RDONLY)
    stderr = (evidence_root / 'anchor.stderr').open('wb')
    anchor = subprocess.Popen([
        sys.executable, '-B', str(SOURCE), '--anchor', str(control_r),
        str(event_w), str(native_stdin), '--', *command,
    ], pass_fds=(control_r, event_w, native_stdin), start_new_session=True,
       stdout=subprocess.DEVNULL, stderr=stderr)
    for fd in (control_r, event_w, native_stdin):
        os.close(fd)
    try:
        yield anchor, control_w, event_r
    finally:
        for fd in (control_w, event_r):
            try:
                os.close(fd)
            except OSError:
                pass
        anchor.wait(timeout=5)
        # Helpers in these fault controls self-expire. Never signal a group
        # after losing its anchor, including when checking the old broken code.
        deadline = time.monotonic() + 4
        while owner.Kernel().session(anchor.pid) and time.monotonic() < deadline:
            time.sleep(.01)
        stderr.close()


def spawned(event_r):
    ready, _, _ = select.select([event_r], [], [], 2)
    if not ready:
        raise AssertionError('anchor did not report native spawn')
    value = json.loads(os.read(event_r, 4096).splitlines()[0])
    if value['event'] != 'spawn':
        raise AssertionError(value)
    return value


def assert_revoked(test, anchor):
    code = anchor.wait(timeout=2)
    deadline = time.monotonic() + .5
    while owner.Kernel().session(anchor.pid) and time.monotonic() < deadline:
        time.sleep(.01)
    test.assertEqual(owner.Kernel().session(anchor.pid), [])
    test.assertEqual(code, -signal.SIGKILL)


def read_event(fd):
    line = b''
    deadline = time.monotonic() + 2
    while not line.endswith(b'\n'):
        ready, _, _ = select.select([fd], [], [], max(0, deadline - time.monotonic()))
        if not ready:
            raise AssertionError('missing anchor event')
        chunk = os.read(fd, 1)
        if not chunk:
            raise AssertionError('anchor event pipe closed')
        line += chunk
    return json.loads(line)


def wait_file(path):
    deadline = time.monotonic() + 2
    while not path.exists() and time.monotonic() < deadline:
        time.sleep(.01)
    if not path.exists():
        raise AssertionError('native readiness not observed')


@contextlib.contextmanager
def controlled_guardian(root, native):
    control_r, control_w = os.pipe()
    evidence = root / 'lifecycle.jsonl'
    guardian = subprocess.Popen([
        sys.executable, '-B', str(SOURCE), '--evidence', str(evidence),
        '--cleanup-seconds', '1', '--native-control-fd', str(control_r), '--',
        sys.executable, '-B', '-c', native, str(control_r),
    ], pass_fds=(control_r,), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    os.close(control_r)
    try:
        yield guardian, control_w, evidence
    finally:
        try:
            os.close(control_w)
        except OSError:
            pass
        guardian.communicate(timeout=5)


class OwnershipTests(unittest.TestCase):
    def test_reparenting_does_not_change_birth_but_pid_reuse_does(self):
        born = dict(pid=101, birth=[10, 42], ppid=100, pgid=100)
        self.assertTrue(owner.same_process(born, dict(born, ppid=1)))
        self.assertFalse(owner.same_process(born, dict(born, birth=[11, 42])))

    def test_live_anchor_preserves_session_ownership_after_parent_exit(self):
        anchor = dict(pid=100, birth=[10, 40], ppid=99, pgid=100)
        orphan = dict(pid=102, birth=[10, 42], ppid=1, pgid=100)
        self.assertEqual(owner.owned_group(anchor, anchor, [anchor, orphan]), [orphan])
        with self.assertRaisesRegex(RuntimeError, 'lost or replaced'):
            owner.owned_group(anchor, None, [orphan])
        with self.assertRaisesRegex(RuntimeError, 'lost or replaced'):
            owner.owned_group(anchor, dict(anchor, birth=[11, 0]), [orphan])
        with self.assertRaisesRegex(RuntimeError, 'ownership unresolved'):
            owner.owned_group(anchor, anchor, [anchor, dict(orphan, pgid=102)])

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_actual_fast_parent_exit_reaps_reparented_helper_and_preserves_native_result(self):
        for code in (0, 7):
            with self.subTest(code=code), tempfile.TemporaryDirectory() as root:
                evidence = Path(root) / 'lifecycle.jsonl'
                native = ('import subprocess,sys; '
                          'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"]); '
                          'print("native-output",flush=True); '
                          'sys.exit(' + str(code) + ')')
                result = subprocess.run([sys.executable, str(SOURCE), '--evidence', str(evidence),
                                         '--cleanup-seconds', '1', '--', sys.executable, '-c', native],
                                        capture_output=True, timeout=10)
                self.assertEqual(result.returncode, code, result.stderr.decode())
                self.assertEqual(result.stdout, b'native-output\n')
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertEqual(rows[-1]['native_exit'], code)
                self.assertTrue(rows[-1]['cleanup_complete'], rows)
                anchor = rows[0]['identity']['pid']
                self.assertEqual(owner.Kernel().session(anchor), [])
                self.assertEqual(evidence.stat().st_mode & 0o777, 0o600)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_stubborn_helper_uses_one_owned_final_kill_without_changing_successful_native_exit(self):
        with tempfile.TemporaryDirectory() as root:
            evidence = Path(root) / 'lifecycle.jsonl'
            ready = Path(root) / 'ready'
            helper = ('import signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); '
                      'pathlib.Path(' + repr(str(ready)) + ').touch(); time.sleep(30)')
            native = ('import subprocess,sys,time,pathlib; '
                      'subprocess.Popen([sys.executable,"-c",' + repr(helper) + ']); '
                      '\nwhile not pathlib.Path(' + repr(str(ready)) + ').exists(): time.sleep(.01)')
            result = subprocess.run([sys.executable, str(SOURCE), '--evidence', str(evidence),
                                     '--cleanup-seconds', '.2', '--', sys.executable, '-c', native],
                                    capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            rows = [json.loads(line) for line in evidence.read_text().splitlines()]
            self.assertEqual(sum(row['event'] == 'kill_owned_session' for row in rows), 1)
            self.assertTrue(rows[-1]['cleanup_complete'], rows)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_guardian_death_revokes_anchor_lease(self):
        with tempfile.TemporaryDirectory() as root:
            evidence = Path(root) / 'lifecycle.jsonl'
            guardian = subprocess.Popen([sys.executable, str(SOURCE), '--evidence', str(evidence),
                '--', sys.executable, '-c', 'import time; time.sleep(30)'],
                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 5
                rows = []
                while time.monotonic() < deadline:
                    rows = [json.loads(line) for line in evidence.read_text().splitlines()] if evidence.exists() else []
                    if any(row['event'] == 'spawn' for row in rows):
                        break
                    time.sleep(.01)
                self.assertTrue(any(row['event'] == 'spawn' for row in rows), rows)
                anchor = rows[0]['identity']['pid']
                guardian.kill()
                guardian.wait(timeout=2)
                deadline = time.monotonic() + 2
                while owner.Kernel().session(anchor) and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertEqual(owner.Kernel().session(anchor), [])
            finally:
                if guardian.poll() is None:
                    guardian.terminate()
                    guardian.wait(timeout=5)
                guardian.stderr.close()


    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_guardian_loss_before_start_revokes_anchor_without_spawning(self):
        with tempfile.TemporaryDirectory() as root:
            marker = Path(root) / 'must-not-spawn'
            command = [sys.executable, '-B', '-c',
                       'from pathlib import Path; Path(' + repr(str(marker)) + ').touch()']
            with direct_anchor(command, Path(root)) as (anchor, control, _events):
                os.close(control)
                assert_revoked(self, anchor)
                self.assertFalse(marker.exists())

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_guardian_loss_at_spawn_report_revokes_already_started_session(self):
        with tempfile.TemporaryDirectory() as root:
            command = [sys.executable, '-B', '-c', 'import time; time.sleep(3)']
            with direct_anchor(command, Path(root)) as (anchor, control, events):
                # S is already buffered when both guardian pipe ends disappear.
                # The spawn write deterministically raises BrokenPipe before
                # the anchor can observe lease EOF on its next control read.
                os.close(events)
                os.write(control, b'S')
                os.close(control)
                assert_revoked(self, anchor)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_failed_exit_report_revokes_reparented_helper(self):
        with tempfile.TemporaryDirectory() as root:
            release = Path(root) / 'release-native'
            native = ('import pathlib,subprocess,sys,time; '
                      'subprocess.Popen([sys.executable,"-B","-c","import time; time.sleep(3)"]); '
                      '\nwhile not pathlib.Path(' + repr(str(release)) + ').exists(): time.sleep(.01)')
            with direct_anchor([sys.executable, '-B', '-c', native], Path(root)) as (anchor, control, events):
                os.write(control, b'S')
                spawned(events)
                os.close(events)
                release.touch()
                # Even with the lease still open, losing the event sink cannot
                # abandon the live session when reporting native completion.
                assert_revoked(self, anchor)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_native_spawn_failure_revokes_anchor(self):
        with tempfile.TemporaryDirectory() as root:
            with direct_anchor([str(Path(root) / 'absent-native')], Path(root)) as (anchor, control, _events):
                os.write(control, b'S')
                assert_revoked(self, anchor)


    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_guardian_cancellation_preserves_native_exit_and_completes_cleanup(self):
        with tempfile.TemporaryDirectory() as root:
            evidence = Path(root) / 'lifecycle.jsonl'
            guardian = subprocess.Popen([
                sys.executable, '-B', str(SOURCE), '--evidence', str(evidence),
                '--cleanup-seconds', '1', '--', sys.executable, '-B', '-c',
                'import time; time.sleep(3)',
            ], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 2
                rows = []
                while time.monotonic() < deadline:
                    rows = [json.loads(line) for line in evidence.read_text().splitlines()] if evidence.exists() else []
                    if any(row['event'] == 'spawn' for row in rows):
                        break
                    time.sleep(.01)
                self.assertTrue(any(row['event'] == 'spawn' for row in rows), rows)
                guardian.terminate()
                self.assertEqual(guardian.wait(timeout=3), 128 + signal.SIGTERM)
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertTrue(rows[-1]['cancelled'])
                self.assertTrue(rows[-1]['cleanup_complete'])
                self.assertEqual(rows[-1]['native_exit'], -signal.SIGTERM)
            finally:
                if guardian.poll() is None:
                    guardian.terminate()
                    guardian.wait(timeout=5)
                guardian.stderr.close()

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_private_interrupt_preserves_native_stdio_and_does_not_inherit_control(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / 'native-ready'
            native = (
                'import os,pathlib,signal,sys,time\n'
                'try: os.fstat(int(sys.argv[1])); raise AssertionError("control inherited")\n'
                'except OSError: pass\n'
                'def stop(*_):\n'
                ' print("native-interrupted",flush=True)\n'
                ' sys.exit(130)\n'
                'signal.signal(signal.SIGINT,stop)\n'
                'print("echo:"+sys.stdin.readline().rstrip("\\n"),flush=True)\n'
                'pathlib.Path(' + repr(str(ready)) + ').touch()\n'
                'time.sleep(3)\n')
            with controlled_guardian(root, native) as (guardian, control, evidence):
                guardian.stdin.write(b'unchanged-native-input\n')
                guardian.stdin.flush()
                wait_file(ready)
                os.write(control, b'I')
                stdout, stderr = guardian.communicate(timeout=4)
                self.assertEqual(guardian.returncode, 130, stderr.decode())
                self.assertEqual(stdout, b'echo:unchanged-native-input\nnative-interrupted\n')
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertEqual(sum(r['event'] == 'native_interrupt_dispatched' for r in rows), 1)
                self.assertFalse(rows[-1]['cancelled'], rows)
                self.assertEqual(rows[-1]['native_exit'], 130)
                self.assertTrue(rows[-1]['cleanup_complete'], rows)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_interrupt_after_native_exit_is_rejected_while_anchor_remains_live(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release = root / 'native-release'
            native = ('import pathlib,time\nwhile not pathlib.Path(' + repr(str(release)) +
                      ').exists(): time.sleep(.01)')
            with direct_anchor([sys.executable, '-B', '-c', native], root) as (anchor, control, events):
                os.write(control, b'S')
                self.assertEqual(read_event(events)['event'], 'spawn')
                release.touch()
                self.assertEqual(read_event(events), {'event': 'exit', 'code': 0})
                os.write(control, b'I')
                self.assertEqual(read_event(events), {
                    'event': 'native_interrupt_rejected', 'reason': 'native_exited'})
                os.write(control, b'R')
                self.assertEqual(anchor.wait(timeout=2), 0)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_guardian_sigint_is_cleanup_not_native_interrupt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready, native_stop = root / 'ready', root / 'native-stop'
            native = ('import pathlib,signal,time\n'
                      'signal.signal(signal.SIGINT,lambda *_: pathlib.Path(' + repr(str(native_stop)) +
                      ').touch())\npathlib.Path(' + repr(str(ready)) + ').touch()\ntime.sleep(3)')
            with controlled_guardian(root, native) as (guardian, _control, evidence):
                wait_file(ready)
                guardian.send_signal(signal.SIGINT)
                guardian.communicate(timeout=4)
                self.assertEqual(guardian.returncode, 128 + signal.SIGTERM)
                self.assertFalse(native_stop.exists())
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertFalse(any(r['event'] == 'native_interrupt_dispatched' for r in rows))
                self.assertTrue(rows[-1]['cancelled'])
                self.assertTrue(rows[-1]['cleanup_complete'])

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_outer_control_eof_revokes_session_without_claiming_native_stop(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / 'ready'
            native = ('import pathlib,subprocess,sys,time\n'
                      'subprocess.Popen([sys.executable,"-B","-c","import time;time.sleep(3)"])\n'
                      'pathlib.Path(' + repr(str(ready)) + ').touch()\ntime.sleep(3)')
            with controlled_guardian(root, native) as (guardian, control, evidence):
                wait_file(ready)
                os.close(control)
                guardian.communicate(timeout=4)
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertEqual(sum(r['event'] == 'native_control_eof' for r in rows), 1)
                self.assertFalse(any(r['event'] == 'native_interrupt_dispatched' for r in rows))
                self.assertTrue(rows[-1]['cancelled'])
                self.assertTrue(rows[-1]['cleanup_complete'])
                self.assertEqual(owner.Kernel().session(rows[0]['identity']['pid']), [])

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_outer_controller_death_closes_its_private_lease(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence, ready = root / 'lifecycle.jsonl', root / 'native-ready'
            native = 'import pathlib,time;pathlib.Path(' + repr(str(ready)) + ').touch();time.sleep(3)'
            controller_code = (
                'import os,subprocess,sys,time\nr,w=os.pipe()\n'
                'p=subprocess.Popen([sys.executable,"-B",' + repr(str(SOURCE)) +
                ',"--evidence",' + repr(str(evidence)) +
                ',"--cleanup-seconds","1","--native-control-fd",str(r),"--",sys.executable,"-B","-c",' +
                repr(native) + '],pass_fds=(r,),stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n'
                'os.close(r)\ntime.sleep(3)')
            controller = subprocess.Popen([sys.executable, '-B', '-c', controller_code])
            try:
                wait_file(ready)
                controller.kill()
                controller.wait(timeout=2)
                deadline = time.monotonic() + 3
                rows = []
                while time.monotonic() < deadline:
                    rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                    if rows[-1]['event'] == 'result':
                        break
                    time.sleep(.01)
                self.assertEqual(rows[-1]['event'], 'result', rows)
                self.assertTrue(rows[-1]['cancelled'])
                self.assertTrue(rows[-1]['cleanup_complete'])
                self.assertFalse(any(r['event'] == 'native_interrupt_dispatched' for r in rows))
                self.assertEqual(owner.Kernel().session(rows[0]['identity']['pid']), [])
            finally:
                if controller.poll() is None:
                    controller.kill()
                    controller.wait(timeout=2)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS session owner')
    def test_unknown_private_command_revokes_instead_of_signalling_native(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ready = root / 'ready'
            native = 'import pathlib,time;pathlib.Path(' + repr(str(ready)) + ').touch();time.sleep(3)'
            with controlled_guardian(root, native) as (guardian, control, evidence):
                wait_file(ready)
                os.write(control, b'X')
                guardian.communicate(timeout=4)
                rows = [json.loads(line) for line in evidence.read_text().splitlines()]
                self.assertEqual(guardian.returncode, 125)
                self.assertIn('invalid native control command', rows[-1]['error'])
                self.assertFalse(any(r['event'] == 'native_interrupt_dispatched' for r in rows))
                self.assertEqual(owner.Kernel().session(rows[0]['identity']['pid']), [])


if __name__ == '__main__':
    unittest.main()
