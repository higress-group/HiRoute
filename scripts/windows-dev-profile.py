#!/usr/bin/env python3
"""Inspect and reversibly archive orphaned HiRoute data for native validation."""
import hashlib
import json
import os
import re
from pathlib import Path
import stat


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def extended_windows_path(value):
    if value.startswith('\\\\?\\'):
        return value
    if value.startswith('\\\\'):
        return '\\\\?\\UNC\\' + value[2:]
    return '\\\\?\\' + value


def plain(path):
    path = Path(path)
    for entry in (path, *path.parents):
        try:
            info = entry.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 1024:
            raise ValueError('Reparse paths cannot be inspected or archived')
    return path


def locations(native_home, local_app_data):
    return {'data': native_home / '.local/share/hiroute',
            'state': local_app_data / 'HiRoute/cli',
            'shell_state': local_app_data / 'HiRoute/shell',
            'programs': local_app_data / 'Programs/HiRoute',
            'stable_cli': native_home / '.local/bin/hiroute.exe'}


def inspect_installation(native_home, local_app_data):
    """Read this account's installation facts without starting the product."""
    paths, facts = locations(native_home, local_app_data), {}
    for name, path in paths.items():
        plain(path)
        facts[name] = dict(path=str(path), exists=path.exists())
        if path.is_dir():
            facts[name]['entries'] = sorted(p.name for p in path.iterdir())[:50]
        elif path.is_file():
            facts[name].update(sha256=digest(path), bytes=path.stat().st_size)
    marker = plain(paths['data'] / 'standalone.json')
    if marker.is_file():
        if marker.stat().st_size > 64 * 1024:
            raise ValueError('Installation marker exceeds its format bound')
        record = json.loads(marker.read_text(encoding='utf-8'))
        facts['marker'] = dict(path=str(marker), sha256=digest(marker), record=record)
        for name, path in [('installed_cli', Path(record['install_root']) / 'hiroute.exe'),
                           ('definition', Path(record['service_definition']))]:
            plain(path)
            facts[name] = dict(path=str(path), exists=path.exists())
            if path.is_file():
                facts[name].update(sha256=digest(path), bytes=path.stat().st_size)
                if name == 'definition' and path.stat().st_size <= 64 * 1024:
                    facts[name]['text'] = path.read_text(encoding='utf-8', errors='replace')
    for name in ('shell-package.json', 'shell-install.json', 'uninstall.exe'):
        path = plain(paths['programs'] / name)
        if path.is_file():
            facts[name] = dict(path=str(path), sha256=digest(path), bytes=path.stat().st_size)
            if name.endswith('.json') and path.stat().st_size <= 64 * 1024:
                facts[name]['record'] = json.loads(path.read_text(encoding='utf-8'))
    for name in ('install-pending.json', 'package-cleanup.json', 'update-outcome.json'):
        path = plain(local_app_data / 'HiRoute/shell' / name)
        if path.is_file():
            facts[name] = dict(sha256=digest(path), bytes=path.stat().st_size,
                               modified_unix=path.stat().st_mtime)
            if path.stat().st_size <= 64 * 1024:
                facts[name]['record'] = json.loads(path.read_text(encoding='utf-8'))
    journal = plain(paths['data'] / 'service/install-journal.json')
    if journal.is_file():
        facts['install-journal.json'] = dict(sha256=digest(journal), bytes=journal.stat().st_size)
        if journal.stat().st_size <= 64 * 1024:
            facts['install-journal.json']['record'] = json.loads(journal.read_text(encoding='utf-8'))
    diagnostics = plain(paths['state'] / 'diagnostics')
    tails = {}
    if diagnostics.is_dir():
        for path in diagnostics.rglob('*.jsonl'):
            plain(path)
            if len(tails) == 16:
                break
            if path.is_file():
                size = path.stat().st_size
                with path.open('rb') as stream:
                    stream.seek(max(0, size - 64 * 1024))
                    lines = stream.read(64 * 1024).decode('utf-8', errors='replace').splitlines()
                records = []
                for line in lines:
                    try:
                        records.append(json.loads(line))
                    except ValueError:
                        pass  # A concurrently written or clipped first/last record is incomplete.
                tails[str(path.relative_to(diagnostics))] = dict(bytes=size, records=records)
    facts['diagnostics_tail'] = tails
    # Pre-replacement refusal may precede the transaction journal. Inspect only
    expected = facts.get('shell-package.json', {}).get('record', {}).get('files', {})
    mismatches = []
    for name, wanted in expected.items():
        if (len(name) >= 512 or '\\' in name or any(p in ('', '.', '..') for p in name.split('/'))):
            raise ValueError('Unsafe shell inventory path')
        path = plain(paths['programs'] / name)
        if path.exists() and digest(path) != wanted:
            mismatches.append(name)
    if expected:
        facts['shell_inventory_mismatches'] = mismatches
    backups = plain(local_app_data / 'HiRoute/shell/install-backups')
    if backups.is_dir():
        facts['shell_backup_entries'] = sorted(str(p.relative_to(backups)) for p in backups.rglob('*') if p.is_file())[:100]
        facts['shell_backup_mismatches'] = [name for name, wanted in expected.items()
                                           if (path := plain(backups / name)).is_file() and digest(path) != wanted]
    return facts


def inventory(root):
    """Reject links/special entries before walking or hashing any child."""
    if os.name == 'nt':
        root = Path(extended_windows_path(os.path.abspath(root)))
    def fail(error):
        raise error
    rows = {}
    for directory, children, files in os.walk(plain(root), followlinks=False, onerror=fail):
        for path in (Path(directory), *(Path(directory) / n for n in children + files)):
            plain(path)
            info = path.lstat()
            key = str(path.relative_to(root))
            if stat.S_ISDIR(info.st_mode):
                rows[key] = {'kind': 'directory'}
            elif stat.S_ISREG(info.st_mode):
                rows[key] = dict(kind='file', bytes=info.st_size, sha256=digest(path))
            else:
                raise ValueError('Unknown filesystem entry; retain the original data')
    return rows


def candidate_uninstaller(native_home, local_app_data, inspected, product, shell_sha256, activity):
    """Admit only the measured candidate's normal, data-preserving NSIS entry."""
    if activity.get('administrator_groups') is not False:
        raise ValueError('Candidate cleanup requires a standard account')
    current = inspect_installation(native_home, local_app_data)
    if current != inspected:
        raise ValueError('Installation changed since inspection')
    marker = current.get('marker', {}).get('record', {})
    install = Path(marker.get('install_root', ''))
    roots = locations(native_home, local_app_data)
    if (marker.get('schema_version') != 'hiroute.standalone-install/v1'
            or marker.get('target') != product['target']
            or install.parent != native_home / '.local/lib/hiroute'
            or not install.name.endswith('-' + product['source_revision'][:12])):
        raise ValueError('Installed product differs from the selected candidate')
    for name in ('hiroute.exe', 'hirouted.exe'):
        if digest(plain(install / name)) != product['files'][name]['sha256']:
            raise ValueError('Installed binary differs from the selected candidate')
    shell = plain(roots['programs'] / 'hiroute-shell.exe')
    uninstaller = plain(roots['programs'] / 'uninstall.exe')
    expected = current.get('shell-package.json', {}).get('record', {}).get('files', {})
    if (digest(shell) != shell_sha256 or expected.get('hiroute-shell.exe') != shell_sha256
            or digest(uninstaller) != expected.get('uninstall.exe')):
        raise ValueError('Uninstaller or shell differs from its measured inventory')
    definition = json.loads(current['definition']['text'])
    if definition.get('manager') != 'task':
        raise ValueError('Only the owned Task candidate can be removed')
    if any(Path(p.get('ExecutablePath') or '') != install / 'hirouted.exe'
           for p in activity.get('processes', [])):
        raise ValueError('Another product or installer is active; preserve it')
    if any(t.get('TaskName') != definition['name'] for t in activity.get('tasks', [])):
        raise ValueError('Another validation or product Task is active; preserve it')
    return uninstaller, install


def require_fresh_nsis_packages(native_home):
    packages = plain(native_home / '.local/lib/hiroute')
    if packages.exists() and (not packages.is_dir() or next(packages.iterdir(), None) is not None):
        raise ValueError('Fresh NSIS acceptance requires no retained package roots; use the Rust cleanup entry first')


def archive_orphaned_data(native_home, local_app_data, destination, inspected, activity):
    """Preserve every byte; installed products and live/managed data are refused."""
    if activity.get('administrator_groups') is not False:
        raise ValueError('A standard account without Administrators membership is required')
    if activity.get('processes') or activity.get('tasks'):
        raise ValueError('HiRoute activity remains; use the owning product stop/uninstall entry')
    current = inspect_installation(native_home, local_app_data)
    if current != inspected:
        raise ValueError('Installation facts changed since inspection; inspect again')
    if 'marker' in current or current['programs']['exists'] or current['stable_cli']['exists']:
        raise ValueError('An installed product remains; use its normal uninstall entry first')
    paths = locations(native_home, local_app_data)
    # Moving a managed artifact would invalidate its native Agent reference.
    archived = ('data', 'state', 'shell_state')
    roots = [str(paths[n]).replace('/', '\\').casefold() for n in archived]
    for name in ('.codex/config.toml', '.claude/settings.json',
                 '.agents/skills/hiroute-collaboration/SKILL.md'):
        path = plain(native_home / name)
        if path.is_file():
            if path.stat().st_size > 4 * 1024 * 1024:
                raise ValueError('Native configuration needs manual inspection')
            text = path.read_text(encoding='utf-8').replace('\\\\', '\\').replace('/', '\\').casefold()
            if any(root in text for root in roots):
                raise ValueError('Native Agent configuration still references the old data; restore it first')
    plans = []
    for name in archived:
        source, target = paths[name], plain(destination / ('previous-' + name))
        if not source.exists():
            continue
        if not source.is_dir():
            raise ValueError('Unexpected installation data entry; preserve it')
        if target.exists() or source.stat().st_dev != destination.stat().st_dev:
            raise ValueError('Backup needs an unused location on the same volume')
        plans.append((source, target, inventory(source)))
    record = dict(operation='archive_orphaned_installation_data', native_configuration='unchanged',
                  paths=[dict(source=str(s), backup=str(t), inventory=rows) for s, t, rows in plans])
    manifest = destination / 'profile-backup.json'
    with manifest.open('x', encoding='utf-8') as stream:
        stream.write(json.dumps(record, indent=2))
        stream.flush()
        os.fsync(stream.fileno())
    moved = []
    try:
        for source, target, expected in plans:
            if inventory(source) != expected:
                raise ValueError('Data changed before archival; preserve it')
            source.rename(target)
            moved.append((source, target))
            if inventory(target) != expected:
                raise ValueError('Archived data differs; restore the preserved directory')
    except Exception:
        for source, target in reversed(moved):
            if not source.exists():
                target.rename(source)
        (destination / 'profile-backup-outcome.json').write_text(json.dumps(dict(
            outcome='restored_after_error', manifest_sha256=digest(manifest))), encoding='utf-8')
        raise
    (destination / 'profile-backup-outcome.json').write_text(json.dumps(dict(
        outcome='archived_and_verified', manifest_sha256=digest(manifest))), encoding='utf-8')
    return dict(archived=[str(t) for _, t in moved], backup_manifest='profile-backup.json',
                native_configuration='unchanged', assessment='preparation_only')


def bounded_json(path, maximum=1024 * 1024):
    path = plain(path)
    if not path.is_file() or path.stat().st_size > maximum:
        raise ValueError('Missing or oversized recovery metadata')
    with path.open('rb') as stream:
        content = stream.read(maximum + 1)
    if len(content) > maximum:
        raise ValueError('Recovery metadata changed while reading')
    return json.loads(content)
