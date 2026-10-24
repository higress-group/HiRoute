#!/usr/bin/env python3
"""Own a native acceptance process session until every session member is gone.

The guardian stays outside the native session. Its anchor stays alive inside it,
even when the client exits first. A private pipe leases that anchor: guardian
death closes the lease and kills the anchored group. Native stdout/stdin remain
untouched. This is a macOS acceptance facility, not a production client launcher.
"""
import argparse
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


def same_process(left, right):
    # PPID and process group are relationships, not process birth identity.
    return bool(left and right) and all(left[k] == right[k] for k in ('pid', 'birth'))


class Kernel:
    def __init__(self):
        spec = importlib.util.spec_from_file_location('native_scope', Path(__file__).with_name('native-agent-scope.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.observer = module.ExactProcessObserver()
        self.lib = self.observer.lib
        self.lib.proc_listallpids.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.lib.proc_listallpids.restype = ctypes.c_int

    def snapshot(self, pid):
        row = self.observer.snapshot([pid])[0]
        return row if row['exists'] else None

    def session(self, sid):
        count = self.lib.proc_listallpids(None, 0)
        if not 0 <= count < 31744:
            raise RuntimeError('process inventory bound')
        values = (ctypes.c_int * (count + 1024))()
        count = self.lib.proc_listallpids(values, ctypes.sizeof(values))
        if not 0 <= count <= len(values):
            raise RuntimeError('process inventory overflow')
        rows = []
        for pid in values[:count]:
            if pid <= 0:
                continue
            try:
                if os.getsid(pid) != sid:
                    continue
                row = self.snapshot(pid)
                if row and os.getsid(pid) == sid:
                    rows.append(row)
            except ProcessLookupError:
                continue
        return rows


def owned_group(anchor, current, members):
    """A live new-session anchor prevents PGID/SID reuse and foreign admission.

    Kernel session membership, established by this launch, survives reparenting.
    A descendant-created group is reported as unsupported instead of signalled.
    """
    if not same_process(anchor, current) or current['pgid'] != anchor['pid']:
        raise RuntimeError('session anchor lost or replaced')
    if any(row['pgid'] != anchor['pid'] for row in members):
        raise RuntimeError('descendant changed process group; ownership unresolved')
    return [row for row in members if row['pid'] != anchor['pid']]


def anchor_main(control, events, native_stdin, command):
    if os.getsid(0) != os.getpid() or os.getpgrp() != os.getpid():
        raise RuntimeError('anchor requires a new session')
    signal.signal(signal.SIGTERM, lambda *_: None)
    signal.signal(signal.SIGINT, lambda *_: None)
    def report(value):
        os.write(events, (json.dumps(value) + '\n').encode())
    released = False
    try:
        # Spawn only after the guardian has retained the live anchor's birth identity.
        if os.read(control, 1) != b'S':
            return 125
        child = subprocess.Popen(command, stdin=native_stdin, close_fds=True)
        os.close(native_stdin)
        report({'event': 'spawn', 'pid': child.pid})
        reported = False
        while True:
            code = child.poll()
            if code is not None and not reported:
                report({'event': 'exit', 'code': code})
                reported = True
            ready, _, _ = select.select([control], [], [], 0.02)
            if ready:
                message = os.read(control, 1)
                if message == b'I':
                    # This single thread owns and reaps the child. Between the
                    # alive check and kill, an exited but unreaped child cannot
                    # have its PID reused. No process-name/ancestry inference.
                    if child.poll() is None:
                        try:
                            os.kill(child.pid, signal.SIGINT)
                            report({'event': 'native_interrupt_dispatched', 'pid': child.pid})
                        except ProcessLookupError:
                            report({'event': 'native_interrupt_rejected', 'reason': 'native_exited'})
                    else:
                        report({'event': 'native_interrupt_rejected', 'reason': 'native_exited'})
                if message == b'R' and reported:
                    released = True
                    return 0
                if not message:
                    return 125
    finally:
        # Only the guardian's validated release permits this session anchor to
        # leave normally. EOF, spawn failure and a broken event pipe must all
        # revoke the same lease, including failures before the next control read.
        # This still-live session leader cannot be reused; signal only its group.
        # Descendants that leave that group remain explicitly unsupported.
        if not released:
            os.killpg(os.getpid(), signal.SIGKILL)


def supervise(command, evidence, seconds, native_control=None):
    kernel = Kernel()
    events = []
    cancelled = []
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda number, _frame: cancelled.append(number))
    control_r, control_w = os.pipe()
    event_r, event_w = os.pipe()
    native_stdin = os.dup(0)
    anchor = None
    native_exit = None
    complete = False
    failure = None
    descriptor = os.open(evidence, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    def record(value):
        events.append(value)
        os.write(descriptor, (json.dumps(value) + '\n').encode())
    try:
        anchor = subprocess.Popen([
            sys.executable, str(Path(__file__).resolve()), '--anchor',
            str(control_r), str(event_w), str(native_stdin), '--', *command,
        ], pass_fds=(control_r, event_w, native_stdin), start_new_session=True)
        for fd in (control_r, event_w, native_stdin):
            os.close(fd)
        birth = kernel.snapshot(anchor.pid)
        if not birth or birth['pgid'] != anchor.pid or os.getsid(anchor.pid) != anchor.pid:
            raise RuntimeError('new session identity unavailable')
        record({'event': 'anchor', 'identity': birth})
        os.write(control_w, b'S')
        pending = b''
        deadline = None
        terminated = False
        while True:
            watched = [event_r] + ([native_control] if native_control is not None else [])
            ready, _, _ = select.select(watched, [], [], 0.02)
            if event_r in ready:
                data = os.read(event_r, 4096)
                if not data and anchor.poll() is not None:
                    raise RuntimeError('anchor exited before release')
                pending += data
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    value = json.loads(line)
                    record(value)
                    if value['event'] == 'exit':
                        native_exit = value['code']
            if native_control in ready:
                message = os.read(native_control, 1)
                if not message:
                    # The optional outer controller leases the whole session.
                    # Losing it is cleanup, never evidence of a native Stop.
                    record({'event': 'native_control_eof'})
                    cancelled.append('native_control_eof')
                    os.close(native_control)
                    native_control = None
                elif message != b'I':
                    raise RuntimeError('invalid native control command')
                elif cancelled or native_exit is not None:
                    record({'event': 'native_interrupt_rejected', 'reason': 'native_exited_or_cleanup'})
                else:
                    os.write(control_w, b'I')
            if native_exit is None and not cancelled:
                continue
            if deadline is None:
                deadline = time.monotonic() + seconds
            current = kernel.snapshot(anchor.pid)
            members = kernel.session(anchor.pid)
            remaining = owned_group(birth, current, members)
            if not remaining and native_exit is not None:
                os.write(control_w, b'R')
                anchor.wait(timeout=max(0.01, deadline - time.monotonic()))
                complete = anchor.returncode == 0
                break
            if not terminated:
                record({'event': 'terminate_owned_session', 'members': members})
                os.killpg(anchor.pid, signal.SIGTERM)
                terminated = True
            if time.monotonic() >= deadline:
                # Revalidate the anchor immediately before the single final kill.
                owned_group(birth, kernel.snapshot(anchor.pid), kernel.session(anchor.pid))
                record({'event': 'kill_owned_session'})
                os.killpg(anchor.pid, signal.SIGKILL)
                anchor.wait()
                # The anchor is gone: observation only, never another group signal.
                complete = not kernel.session(anchor.pid)
                break
    except Exception as error:
        failure = type(error).__name__ + ': ' + str(error)
    finally:
        # EOF revokes the lease even after a guardian-side exception.
        os.close(control_w)
        if native_control is not None:
            os.close(native_control)
        if anchor is not None and anchor.poll() is None:
            anchor.wait(timeout=seconds)
        os.close(event_r)
        record({'event': 'result', 'native_exit': native_exit, 'cleanup_complete': complete,
                'cancelled': bool(cancelled), 'error': failure})
        os.close(descriptor)
    if not complete or failure or native_exit is None:
        return 125
    return native_exit if native_exit >= 0 else 128 - native_exit


def main():
    if sys.argv[1:2] == ['--anchor']:
        return anchor_main(*map(int, sys.argv[2:5]), sys.argv[6:])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', required=True)
    parser.add_argument('--cleanup-seconds', type=float, default=20)
    parser.add_argument('--native-control-fd', type=int,
                        help='private read-only pipe: I dispatches native SIGINT; EOF revokes the session')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if sys.platform != 'darwin' or not 0 < args.cleanup_seconds <= 20:
        parser.error('requires macOS and a cleanup budget within 20 seconds')
    if not command or not Path(command[0]).is_absolute():
        parser.error('requires an absolute native entry')
    if args.native_control_fd is not None:
        fd = args.native_control_fd
        try:
            valid = fd > 2 and stat.S_ISFIFO(os.fstat(fd).st_mode) and (
                fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_ACCMODE) == os.O_RDONLY
        except OSError:
            valid = False
        if not valid:
            parser.error('native control requires a private read-only pipe above standard descriptors')
        # The anchor inherits only its three explicitly named descriptors; the
        # native child inherits none of these control/event/outer-lease pipes.
        os.set_inheritable(fd, False)
    return supervise(command, args.evidence, args.cleanup_seconds, args.native_control_fd)


if __name__ == '__main__':
    raise SystemExit(main())
