#!/usr/bin/env python3
import ctypes
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import stat
import subprocess
import sys
import time
from types import SimpleNamespace
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("capture", Path(__file__).with_name("private-stream-capture.py"))
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class CaptureSessionTest(unittest.TestCase):
    def test_private_provenance_and_expired_session_cleanup(self):
        with tempfile.TemporaryDirectory() as parent:
            binary = Path(parent) / "binary"
            binary.write_bytes(b"fixture binary")
            root = Path(parent).resolve() / "private"
            with patch.object(capture.time, "time", return_value=1000):
                capture.create_session(root, "a" * 40, binary, "fixture", 60)
            self.assertEqual(stat.S_IMODE(root.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((root / "session.json").stat().st_mode), 0o600)
            session = json.loads((root / "session.json").read_text())
            self.assertEqual(session["expires_at"], 1060)
            self.assertEqual(len(session["binary_sha256"]), 64)
            with patch.object(capture.time, "time", return_value=2000):
                with self.assertRaises(ValueError):
                    capture.cleanup(root)
            with capture.capture_lock(root) as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.object(capture.time, "time", return_value=100000):
                    with self.assertRaises(ValueError):
                        capture.cleanup(root)
            # The stable inode remains; only live kernel ownership blocks cleanup.
            with patch.object(capture.time, "time", return_value=100000):
                capture.cleanup(root)
            self.assertFalse(root.exists())

    def test_abnormal_and_forced_exit_release_lock_for_expired_cleanup(self):
        for forced in [False, True]:
            with self.subTest(forced=forced), tempfile.TemporaryDirectory() as parent:
                root = Path(parent).resolve() / "private"
                binary = Path(sys.executable).resolve()
                capture.create_session(root, "a" * 40, binary, "fixture", 1)
                code = ("import os,fcntl,time,sys; from pathlib import Path; "
                        "p=Path(os.environ['HIROUTE_PRIVATE_STREAM_CAPTURE']); "
                        "f=os.open(p/'active.lock',os.O_CREAT|os.O_RDWR,0o600); "
                        "fcntl.flock(f,fcntl.LOCK_EX); (p/'raw.capture').write_bytes(b'raw'); "
                        "print('ready',flush=True); " +
                        ("time.sleep(60)" if forced else "os._exit(3)"))
                with subprocess.Popen([str(binary), "-c", code], stdout=subprocess.PIPE,
                                      env=dict(os.environ, HIROUTE_PRIVATE_STREAM_CAPTURE=str(root)),
                                      text=True) as child:
                    self.assertEqual(child.stdout.readline().strip(), "ready")
                    if forced:
                        child.kill()
                    child.wait(timeout=5)
                self.assertFalse(capture.capture_active(root))
                with patch.object(capture.time, "time", return_value=time.time() + 86401):
                    capture.cleanup(root)
                self.assertFalse(root.exists())

    def test_owner_cleanup_waits_for_delayed_kernel_lock_release(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent).resolve() / "private"
            capture.create_session(root, "a" * 40, Path(sys.executable).resolve(), "fixture", 1)
            with capture.capture_lock(root) as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                timer = threading.Timer(0.1, lambda: fcntl.flock(lock, fcntl.LOCK_UN))
                timer.start()
                try:
                    capture.cleanup(root, require_expired=False, lock_wait_seconds=1)
                finally:
                    timer.join()
            self.assertFalse(root.exists(), "cleanup must wait for the stopped group's lock release")

    def test_owner_cleanup_wait_is_bounded_and_keeps_active_capture(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent).resolve() / "private"
            capture.create_session(root, "a" * 40, Path(sys.executable).resolve(), "fixture", 1)
            with capture.capture_lock(root) as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                started = time.monotonic()
                with self.assertRaisesRegex(ValueError, "capture still owned"):
                    capture.cleanup(root, require_expired=False, lock_wait_seconds=0.1)
                self.assertGreaterEqual(time.monotonic() - started, 0.1)
                self.assertLess(time.monotonic() - started, 2)
                self.assertTrue(root.exists())
                self.assertTrue((root / "session.json").is_file())

    def test_retention_owner_deletes_after_cli_returns_or_graceful_stop(self):
        for early_stop in [False, True]:
            with self.subTest(early_stop=early_stop), tempfile.TemporaryDirectory() as parent:
                root = Path(parent).resolve() / "private"
                binary = Path(sys.executable).resolve()
                capture.create_session(root, "a" * 40, binary, "fixture", 1)
                code = ("import os,fcntl; from pathlib import Path; "
                        "p=Path(os.environ['HIROUTE_PRIVATE_STREAM_CAPTURE']); "
                        "f=os.open(p/'active.lock',os.O_CREAT|os.O_RDWR,0o600); "
                        "fcntl.flock(f,fcntl.LOCK_EX); (p/'raw.capture').write_bytes(b'raw'); "
                        "os._exit(3)")
                args = SimpleNamespace(seconds=1, attempts=1)
                with patch.object(capture, "GROUP_STOP_GRACE_SECONDS", 0.1, create=True):
                    owner = capture.retain_session(args, root, binary, [str(binary), "-c", code],
                                                   "a" * 40, time.time() + (60 if early_stop else 1))
                try:
                    self.assertTrue((root / "raw.capture").exists())
                    if early_stop:
                        os.kill(owner, signal.SIGTERM)
                    deadline = time.monotonic() + 5
                    while root.exists() and time.monotonic() < deadline:
                        time.sleep(0.02)
                    self.assertFalse(root.exists(), "raw retention must be executed without another CLI call")
                finally:
                    if root.exists():
                        os.kill(owner, signal.SIGTERM)
                    os.waitpid(owner, 0)

    @unittest.skipUnless(sys.platform.startswith("linux"), "controlled Linux child subreaper")
    def test_expired_retention_stops_descendant_after_leader_exits(self):
        self.check_descendant_cleanup(leader_exits=True)

    @unittest.skipUnless(sys.platform.startswith("linux"), "controlled Linux child subreaper")
    def test_expired_retention_stops_descendant_after_leader_exits_on_term(self):
        self.check_descendant_cleanup(leader_exits=False)

    @unittest.skipUnless(sys.platform.startswith("linux"), "controlled Linux child subreaper")
    def test_owner_term_during_group_stop_still_kills_descendant(self):
        self.check_descendant_cleanup(leader_exits=False, owner_term_during_stop=True)

    @unittest.skipUnless(sys.platform.startswith("linux"), "controlled Linux child subreaper")
    def test_reader_exit_cannot_skip_owner_cleanup_after_broken_report_and_close(self):
        # Terminate the real CLI reader after the first report. Its owner later
        # reports a natural candidate exit to the now-closed pipe, then closes
        # that same buffered stream. Both failures must still reach deletion.
        libc = ctypes.CDLL(None, use_errno=True)
        previous = ctypes.c_int()
        self.assertEqual(libc.prctl(37, ctypes.byref(previous), 0, 0, 0), 0)
        self.assertEqual(libc.prctl(36, 1, 0, 0, 0), 0)
        cli = owner = candidate = None
        try:
            with tempfile.TemporaryDirectory() as parent:
                root = Path(parent).resolve() / "private"
                binary = Path(sys.executable).resolve()
                capture.create_session(root, "a" * 40, binary, "fixture", 10)
                delete_after = int(time.time()) + 2
                session_path = root / "session.json"
                session = json.loads(session_path.read_text())
                session["delete_after"] = delete_after
                session_path.write_text(json.dumps(session))
                candidate_code = '''import fcntl, os, time
from pathlib import Path
root = Path(os.environ["HIROUTE_PRIVATE_STREAM_CAPTURE"])
fd = os.open(root / "active.lock", os.O_CREAT | os.O_RDWR, 0o600)
fcntl.flock(fd, fcntl.LOCK_EX)
raw = os.open(root / "raw.capture", os.O_CREAT | os.O_WRONLY, 0o600)
os.write(raw, b"synthetic private body")
os.close(raw)
(root.parent / "ready").touch()
while not (root.parent / "finish-candidate").exists():
    time.sleep(0.01)
os._exit(0)
'''
                cli_code = '''import importlib.util, sys
from pathlib import Path
from types import SimpleNamespace
spec = importlib.util.spec_from_file_location("capture", sys.argv[1])
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)
capture.GROUP_STOP_GRACE_SECONDS = 0.1
binary = Path(sys.executable).resolve()
capture.retain_session(SimpleNamespace(seconds=10, attempts=1), Path(sys.argv[2]),
                       binary, [str(binary), "-c", sys.argv[4]], "a" * 40, float(sys.argv[3]))
'''
                cli = subprocess.Popen(
                    [str(binary), "-c", cli_code, capture.__file__, str(root),
                     str(delete_after), candidate_code], stdout=subprocess.PIPE, text=True,
                )
                self.assertTrue(select.select([cli.stdout], [], [], 5)[0], "CLI did not report its owner")
                first = json.loads(cli.stdout.readline())
                owner, candidate = first["retention_owner_pid"], first["pid"]
                deadline = time.monotonic() + 5
                while not (root.parent / "ready").exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue((root / "raw.capture").is_file())
                cli.terminate()
                self.assertEqual(cli.wait(timeout=5), -signal.SIGTERM)
                cli.stdout.close()
                cli = None
                (root.parent / "finish-candidate").touch()
                deadline = time.monotonic() + 5
                while root.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertFalse(root.exists(), "closed CLI report pipe skipped mandatory raw cleanup")
                status = self.wait_for_owned_child(owner)
                owner = None
                self.assertTrue(os.WIFEXITED(status))
                self.assertEqual(os.WEXITSTATUS(status), 1, "report failure must remain a failed owner exit")
        finally:
            if cli is not None:
                if cli.poll() is None:
                    cli.kill()
                cli.wait(timeout=5)
                cli.stdout.close()
            # waitpid proves these are our adopted children before signaling;
            # no process scan or potentially recycled external PID is used.
            for pid in [owner, candidate]:
                if pid is not None:
                    try:
                        completed, _ = os.waitpid(pid, os.WNOHANG)
                        if completed == 0:
                            os.kill(pid, signal.SIGKILL)
                            os.waitpid(pid, 0)
                    except ChildProcessError:
                        pass
            self.assertEqual(libc.prctl(36, previous.value, 0, 0, 0), 0)

    def check_descendant_cleanup(self, leader_exits, owner_term_during_stop=False):
        # Adopt only this test's orphan, so its SIGKILL status and reaping can be
        # asserted without leaving a zombie behind or trusting a recycled PID.
        libc = ctypes.CDLL(None, use_errno=True)
        previous = ctypes.c_int()
        self.assertEqual(libc.prctl(37, ctypes.byref(previous), 0, 0, 0), 0)
        self.assertEqual(libc.prctl(36, 1, 0, 0, 0), 0)
        owner = descendant = None
        try:
            with tempfile.TemporaryDirectory() as parent:
                root = Path(parent).resolve() / "private"
                binary = Path(sys.executable).resolve()
                capture.create_session(root, "a" * 40, binary, "fixture", 1)
                delete_after = int(time.time()) + 2
                session_path = root / "session.json"
                session = json.loads(session_path.read_text())
                session["delete_after"] = delete_after
                session_path.write_text(json.dumps(session))
                code = '''import fcntl, os, signal, sys, time
from pathlib import Path
root = Path(os.environ["HIROUTE_PRIVATE_STREAM_CAPTURE"])
ready = root.parent / "descendant.pid"
if os.fork() == 0:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    fd = os.open(root / "active.lock", os.O_CREAT | os.O_RDWR, 0o600)
    fcntl.flock(fd, fcntl.LOCK_EX)
    (root / "raw.capture").write_bytes(b"synthetic private body")
    ready.write_text(str(os.getpid()))
    while True:
        time.sleep(1)
while not ready.exists():
    time.sleep(0.01)
if sys.argv[1] == "exit":
    os._exit(0)
def terminate(*_):
    if sys.argv[1] == "owner_term":
        os.kill(os.getppid(), signal.SIGTERM)
    os._exit(0)
signal.signal(signal.SIGTERM, terminate)
while True:
    time.sleep(1)
'''
                args = SimpleNamespace(seconds=1, attempts=1)
                mode = "owner_term" if owner_term_during_stop else "exit" if leader_exits else "term"
                with patch.object(capture, "GROUP_STOP_GRACE_SECONDS", 0.1, create=True):
                    owner = capture.retain_session(
                        args, root, binary,
                        [str(binary), "-c", code, mode],
                        "a" * 40, delete_after,
                    )
                descendant = int((root.parent / "descendant.pid").read_text())
                deadline = time.monotonic() + 5
                while root.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertFalse(root.exists(), "raw retention survives the candidate's exited leader")
                status = self.wait_for_owned_child(descendant)
                descendant = None
                self.assertTrue(os.WIFSIGNALED(status))
                self.assertEqual(os.WTERMSIG(status), signal.SIGKILL)
                status = self.wait_for_owned_child(owner)
                owner = None
                self.assertTrue(os.WIFEXITED(status))
                self.assertEqual(os.WEXITSTATUS(status), 1 if owner_term_during_stop else 0)
        finally:
            for pid in [descendant, owner]:
                if pid is not None:
                    try:
                        completed, _ = os.waitpid(pid, os.WNOHANG)
                        if completed == 0:
                            os.kill(pid, signal.SIGKILL)
                            os.waitpid(pid, 0)
                    except ChildProcessError:
                        pass
            self.assertEqual(libc.prctl(36, previous.value, 0, 0, 0), 0)

    def wait_for_owned_child(self, pid):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            completed, status = os.waitpid(pid, os.WNOHANG)
            if completed:
                return status
            time.sleep(0.02)
        self.fail("owned child did not terminate")

    def test_stop_rejects_a_child_in_the_callers_process_group(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            with self.assertRaisesRegex(ValueError, "isolated session/process group"):
                capture.stop(child)
            self.assertIsNone(child.poll())
        finally:
            child.kill()
            child.wait(timeout=5)

    def test_private_writer_does_not_follow_or_overwrite(self):
        with tempfile.TemporaryDirectory() as parent:
            target = Path(parent) / "target"
            target.write_bytes(b"untouched")
            link = Path(parent) / "link"
            link.symlink_to(target)
            with self.assertRaises(FileExistsError):
                capture.write_private(link, b"overwrite")
            self.assertEqual(target.read_bytes(), b"untouched")


if __name__ == "__main__":
    unittest.main()
