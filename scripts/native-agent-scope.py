#!/usr/bin/env python3
"""Prepare an isolated native acceptance command; execution requires --execute.

Client drivers retain their business oracles, ports and owned cleanup. This entry
contains the whole driver and every descendant before any native installation is
launched. Preparation and process observation launch neither HiRoute nor clients.
"""
import argparse
import ctypes
import errno
import json
import os
from pathlib import Path
import re
import sys
import time


HARNESSES = ('codex', 'claude', 'qoder', 'pi', 'dsh', 'opencode', 'hermes')
SANDBOX = Path('/usr/bin/sandbox-exec')
PROTECTED_DAILY = ('.codex/config.toml', '.codex/auth.json', '.codex/sessions',
                   '.claude.json', '.claude/settings.json', '.claude/.credentials.json',
                   '.claude/projects', '.qoder/config.json', '.qoder/settings.json',
                   '.qoder/sessions', '.pi/agent', '.dsh', '.hermes')
PATHS = {'HOME': 'home', 'CODEX_HOME': 'home/.codex', 'CLAUDE_CONFIG_DIR': 'home/.claude',
         'QODER_CONFIG_DIR': 'home/.qoder', 'PI_CODING_AGENT_DIR': 'home/.pi/agent',
         'DSH_HOME': 'home/.dsh', 'HERMES_HOME': 'home/.hermes',
         'XDG_CONFIG_HOME': 'home/.config', 'XDG_DATA_HOME': 'home/.local/share',
         'XDG_CACHE_HOME': 'home/.cache', 'XDG_STATE_HOME': 'home/.local/state',
         'TMPDIR': 'tmp', 'TMP': 'tmp', 'TEMP': 'tmp', 'CLAUDE_CODE_TMPDIR': 'tmp',
         'HIROUTE_RUNTIME_DIR': 'runtime', 'HIROUTE_REPLAY_ROOT': 'replay'}


def private_write(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(data)


def isolated_environment(root, inherited=None):
    """Use a small allowlist, so proxy, credentials and client overrides cannot leak in."""
    inherited = os.environ if inherited is None else inherited
    env = {key: inherited[key] for key in ('LANG', 'LC_ALL', 'LC_CTYPE', 'TERM', 'TZ') if key in inherited}
    env.update({key: str(root / relative) for key, relative in PATHS.items()})
    env.update(PWD=str(root / 'project'), PATH=str(root / 'bin') + ':/usr/bin:/bin:/usr/sbin:/sbin',
               PYTHONDONTWRITEBYTECODE='1', PYTHONNOUSERSITE='1')
    return env


def sandbox_policy(root, daily_home, read_only):
    """Exact dependency exceptions; directory ancestors expose metadata only."""
    root, daily_home = Path(root).resolve(), Path(daily_home).resolve()
    if root == daily_home or daily_home in root.parents or root in daily_home.parents:
        raise ValueError('native scope must be independent of daily HOME')
    literals, subpaths = set(), set()
    protected = [daily_home / value for value in PROTECTED_DAILY]
    for value in read_only:
        path = Path(value).resolve(strict=True)
        if path == daily_home or path in daily_home.parents or any(
                path == guard or path in guard.parents or guard in path.parents for guard in protected):
            raise ValueError('read-only dependency includes daily native configuration or sessions')
        (subpaths if path.is_dir() else literals).add(path)
    ancestors = {parent for path in literals | subpaths for parent in path.parents
                 if parent == daily_home or daily_home in parent.parents} - subpaths
    exceptions = '\n'.join('  (require-not (' + kind + ' ' + json.dumps(str(path)) + '))'
                           for kind, paths in (('literal', literals | ancestors), ('subpath', subpaths))
                           for path in sorted(paths))
    policy = ('(version 1)\n(allow default)\n'
              '(deny file-write* (require-all (require-not (subpath ' + json.dumps(str(root)) + '))\n'
              '  (require-not (literal "/dev/null")) (require-not (literal "/dev/tty"))\n'
              '  (require-not (subpath "/dev/fd")) (require-not (literal "/dev/stdout"))\n'
              '  (require-not (literal "/dev/stderr"))))\n'
              '(deny file-read* (require-all (subpath ' + json.dumps(str(daily_home)) + ')\n' + exceptions + '\n))\n'
              '(deny signal (require-not (target same-sandbox)))\n')
    if ancestors:
        directories = ' '.join('(literal ' + json.dumps(str(path)) + ')' for path in sorted(ancestors))
        policy += '(deny file-read-data ' + directories + ')\n(deny file-read-xattr ' + directories + ')\n'
    # The command cannot weaken its own containment after the controller prepares it.
    for name in ('native-isolation.sb', 'native-scope.private.json'):
        policy += '(deny file-write* (literal ' + json.dumps(str(root / name)) + '))\n'
    return policy


def prepare_scope(harness, root, command, *, read_only=(), expose=None, daily_home=None, inherited=None):
    if harness not in HARNESSES:
        raise ValueError('unknown native harness')
    if not command or not Path(command[0]).is_absolute() or not Path(command[0]).is_file():
        raise ValueError('native acceptance command needs an absolute installed entry')
    root = Path(root).resolve()
    daily_home = Path.home() if daily_home is None else Path(daily_home).resolve()
    exposed = {}
    for name, value in ({} if expose is None else expose).items():
        path = Path(value)
        if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_.-]{0,63}', name) or not path.is_absolute() or not path.is_file():
            raise ValueError('exposed tool requires a plain name and absolute installed file')
        exposed[name] = path.resolve(strict=True)
    reads = [*read_only, command[0], *exposed.values()]
    # Resolve and validate before creating the scope. No host/native subprocess probe.
    policy = sandbox_policy(root, daily_home, reads)
    root.mkdir(mode=0o700, exist_ok=False)
    env = isolated_environment(root, inherited)
    for value in {env[key] for key in PATHS} | {str(root / 'project'), str(root / 'bin'), str(root / 'sessions')}:
        Path(value).mkdir(mode=0o700, parents=True, exist_ok=True)
    for name, path in exposed.items():
        (root / 'bin' / name).symlink_to(path)
    private_write(root / 'native-isolation.sb', policy.encode())
    launch = [str(SANDBOX), '-f', str(root / 'native-isolation.sb'), *map(str, command)]
    result = {'harness': harness, 'root': str(root), 'environment': env, 'command': launch,
              'read_only': sorted({str(Path(value).resolve()) for value in reads}),
              'exposed_binaries': {name: str(path) for name, path in exposed.items()},
              'evidence_limit': 'Preparation only; caller-owned cleanup and every native business oracle remain required.'}
    private_write(root / 'native-scope.private.json', (json.dumps(result, indent=2) + '\n').encode())
    return result


class BsdInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint32) for name in
                ('flags', 'status', 'xstatus', 'pid', 'ppid', 'uid', 'gid', 'ruid', 'rgid', 'svuid', 'svgid', 'rfu')]
    _fields_ += [('comm', ctypes.c_char * 16), ('name', ctypes.c_char * 32)]
    _fields_ += [(name, ctypes.c_uint32) for name in ('nfiles', 'pgid', 'jobc', 'tdev', 'tpgid')]
    _fields_ += [('nice', ctypes.c_int32), ('start_sec', ctypes.c_uint64), ('start_usec', ctypes.c_uint64)]


class ExactProcessObserver:
    """Trusted controller-side read-only observation; never dispatch ps to a worker.

    Call only for the previously selected baseline/owned PIDs. This object neither
    inventories unrelated processes nor grants authority to signal an observed PID.
    Permission/ABI errors fail closed; a missing identity cannot become guard success.
    """
    def __init__(self):
        if sys.platform != 'darwin':
            raise RuntimeError('exact native process observation requires macOS')
        self.lib = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        self.lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                         ctypes.c_void_p, ctypes.c_int]
        self.lib.proc_pidinfo.restype = ctypes.c_int
        self.lib.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
        self.lib.proc_pidpath.restype = ctypes.c_int

    def snapshot(self, pids):
        pids = list(pids)
        if len(pids) > 4096 or any(type(pid) is not int or pid <= 0 for pid in pids):
            raise ValueError('bounded positive exact PIDs required')
        rows = []
        for pid in sorted(set(pids)):
            value = BsdInfo()
            ctypes.set_errno(0)
            size = self.lib.proc_pidinfo(pid, 3, 0, ctypes.byref(value), ctypes.sizeof(value))
            if size == 0 and ctypes.get_errno() == errno.ESRCH:
                rows.append({'pid': pid, 'exists': False})
                continue
            if size != ctypes.sizeof(value) or value.pid != pid:
                raise RuntimeError('exact process identity unavailable; guard remains red')
            image = ctypes.create_string_buffer(4096)
            if self.lib.proc_pidpath(pid, image, len(image)) <= 0:
                raise RuntimeError('exact process image unavailable; guard remains red')
            rows.append({'pid': pid, 'exists': True, 'ppid': int(value.ppid), 'pgid': int(value.pgid),
                         'birth': [int(value.start_sec), int(value.start_usec)],
                         'started': time.strftime('%a %b %e %H:%M:%S %Y', time.localtime(value.start_sec)),
                         'image': image.value.decode(errors='strict')})
        return rows

    def guard_process_output(self, pids):
        """Bridge an existing fixed-PID guard's ps-shaped input without launching ps."""
        rows = self.snapshot(pids)
        if any(not row['exists'] for row in rows):
            raise RuntimeError('baseline process exited; guard remains red')
        return ''.join(f"{row['pid']} {row['ppid']} {row['started']} {row['image']}\n" for row in rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--harness', required=True, choices=HARNESSES)
    parser.add_argument('--root', required=True, help='Fresh independent root outside daily HOME')
    parser.add_argument('--read-only', action='append', default=[], help='Exact installed file or dependency directory')
    parser.add_argument('--expose', action='append', default=[], metavar='NAME=/ABSOLUTE/ENTRY',
                        help='Expose only this explicitly selected tool in the private PATH')
    parser.add_argument('--execute', action='store_true', help='Execute only after the caller has allocated its native lease')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if args.execute and (sys.platform != 'darwin' or not SANDBOX.is_file()):
        parser.error('macOS OS containment unavailable; execution refused')
    try:
        exposed = {}
        for value in args.expose:
            name, separator, entry = value.partition('=')
            if not separator or name in exposed:
                raise ValueError('exposed tool names must be explicit and unique')
            exposed[name] = entry
        prepared = prepare_scope(args.harness, args.root, command, read_only=args.read_only, expose=exposed)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    print(json.dumps({'state': 'executing' if args.execute else 'prepared-no-execution',
                      'harness': args.harness, 'root': prepared['root']}), flush=True)
    if args.execute:
        root = Path(prepared['root'])
        # Capture even import, guard and exec failures before a Product exists.
        for number, name in ((1, 'native.stdout.private.bin'), (2, 'native.stderr.private.bin')):
            fd = os.open(root / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            os.dup2(fd, number)
            os.close(fd)
        os.chdir(root / 'project')
        if os.getpgrp() != os.getpid():
            os.setsid()
        os.execve(prepared['command'][0], prepared['command'], prepared['environment'])
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
