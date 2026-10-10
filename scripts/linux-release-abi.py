#!/usr/bin/env python3
"""Fail closed on the ABI of every ELF in the verified distribution archive."""
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent


def baseline():
    return json.loads((ROOT / 'linux-release-baseline.json').read_text())


def inspect(path, target, run=None):
    policy = baseline()
    platform = policy['targets'][target]
    header = path.read_bytes()[:20]
    if (len(header) != 20 or header[:6] != b'\x7fELF\x02\x01'
            or struct.unpack_from('<H', header, 18)[0] != platform['machine']
            or struct.unpack_from('<H', header, 16)[0] not in (2, 3)):
        raise ValueError(f'{path.name}: invalid ELF architecture/type for {target}')
    if run is None:
        run = lambda *args: subprocess.check_output(args, text=True, env={'PATH': '/usr/bin:/bin', 'LC_ALL': 'C'})
    output = run('readelf', '--wide', '--program-headers', '--dynamic', '--version-info', path)
    interpreters = re.findall(r'Requesting program interpreter:\s*([^\]]+)\]', output)
    if len(interpreters) > 1 or (interpreters and interpreters[0] != platform['interpreter']):
        raise ValueError(f'{path.name}: unsupported ELF loader {interpreters}')
    needed = re.findall(r'\(NEEDED\).*?\[([^\]]+)\]', output)
    forbidden = set(needed) - set(policy['system_libraries'])
    if forbidden:
        raise ValueError(f'{path.name}: unsupported DT_NEEDED {sorted(forbidden)}')
    # Do not permit build-host paths or a newer loader feature to bypass the floor.
    if re.search(r'\((?:RPATH|RUNPATH|RELR|RELRSZ|RELRENT|AUDIT|DEPAUDIT|FILTER|AUXILIARY)\)', output):
        raise ValueError(f'{path.name}: unsupported dynamic loader tag')
    needs = output.split('Version needs section', 1)[-1] if 'Version needs section' in output else ''
    versions = sorted(set(re.findall(r'Name:\s+(\S+)', needs)))
    for version in versions:
        match = re.fullmatch(r'(GLIBC|GLIBCXX|CXXABI|GCC|OPENSSL)_(\d+(?:[._]\d+)*)', version)
        if not match:
            raise ValueError(f'{path.name}: unsupported required symbol version {version}')
        required = tuple(map(int, re.split('[._]', match[2])))
        allowed = tuple(map(int, policy['symbol_versions'][match[1]].split('.')))
        if required > allowed:
            raise ValueError(f'{path.name}: required {version} exceeds {match[1]}_{policy["symbol_versions"][match[1]]}')
    if not interpreters and (needed or versions) and struct.unpack_from('<H', header, 16)[0] == 2:
        raise ValueError(f'{path.name}: dynamic executable has no loader')
    return {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'interpreter': interpreters[0] if interpreters else None,
            'needed': needed, 'required_versions': versions, 'status': 'green'}


def archive(manifest, bundle, packager, run=None):
    verified = packager.verify(manifest, bundle)
    members = packager.safe_members(bundle, verified['files'])
    result = {}
    with tempfile.TemporaryDirectory(prefix='hiroute-archive-abi-') as temporary:
        for name, (data, mode) in members.items():
            executable = name in ('bin/hiroute', 'bin/hirouted', 'libexec/cliproxyapi')
            if not data.startswith(b'\x7fELF') and not executable:
                continue
            path = Path(temporary) / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            try:
                facts = inspect(path, verified['target'], run)
            except ValueError as error:
                raise ValueError(f'archive {name}: {error}') from error
            if facts['sha256'] != verified['files'][name]['sha256']:
                raise ValueError(f'archive identity changed: {name}')
            result[name] = facts
    return {'status': 'green', 'archive_sha256': verified['archive']['sha256'],
            'manifest_sha256': hashlib.sha256(manifest.read_bytes()).hexdigest(),
            'target': verified['target'], 'revision': verified['revision'],
            'baseline': baseline(), 'files': result}
