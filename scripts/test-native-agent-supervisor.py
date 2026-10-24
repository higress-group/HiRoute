#!/usr/bin/env python3
"""Process lifecycle regressions. No Agent, model, service or daily config."""
import importlib.util
import json
import os
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


if __name__ == '__main__':
    unittest.main()
