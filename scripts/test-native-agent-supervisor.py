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


if __name__ == '__main__':
    unittest.main()
