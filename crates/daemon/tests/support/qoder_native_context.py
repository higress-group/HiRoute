"""Qoder fixture facts; product lifecycle remains in the shared Worker journeys.

Borrow an explicitly selected, normally logged-in context. Never copy authentication,
rewrite user settings, or silently substitute the caller's daily HOME. The native
history filename below is a pinned Qoder 1.1.65 fixture fact, not a product reader.
"""
import json
import os
from pathlib import Path
import re
import secrets
import sqlite3
import stat

from native_context_fixture import NativeProxyTrap, digest, prepare, protect_configuration, write_new


def selected_directory(name):
    value = os.environ.get(name)
    assert value, name + ' must explicitly select a normally logged-in Qoder context'
    path = Path(value)
    assert path.is_absolute() and path.is_dir(), name + ' must be an existing absolute directory'
    return path.resolve(strict=True)


def select_context(product):
    home = selected_directory('HIROUTE_QODER_CONTEXT_HOME')
    config = selected_directory('HIROUTE_QODER_CONFIG_DIR')
    product.env.update(HOME=str(home), QODER_CONFIG_DIR=str(config))
    # Qoder's own login state may refresh. Only native settings are part of the
    # byte-preservation promise; no authentication file is opened by this fixture.
    product.qoder_settings_before = settings_snapshot(config)


def settings_snapshot(config):
    result = {}
    for name in ('settings.json', 'settings.local.json'):
        path = Path(config) / name
        assert not path.is_symlink(), 'linked Qoder settings need a separate explicit fixture'
        result[name] = (dict(sha256=digest(path), mode=stat.S_IMODE(path.stat().st_mode))
                        if path.exists() else None)
    return result


def prepare_context(product):
    config = Path(product.env['QODER_CONFIG_DIR'])
    # Private, unique names let a real logged-in context be borrowed without
    # replacing unrelated skills. Cleanup owns exactly these newly created files.
    assert not (config / 'skills').is_symlink(), 'linked user skills are not fixture-owned'
    product.qoder_owned_files = {}
    fixture = prepare(Path(product.env['HOME']), config, product.project, 'qoder',
                      suffix='-' + secrets.token_hex(6), owned_files=product.qoder_owned_files)
    fixture.update(product_storage=str(product.storage),
                   settings_before=product.qoder_settings_before,
                   native_context_setup='explicit-borrowed-login-context')
    product.qoder_fixture = fixture
    return fixture


def assert_settings_preserved(fixture):
    assert settings_snapshot(fixture['config']) == fixture['settings_before'], \
        'Qoder user settings or permissions changed'


def directory_identity(path):
    metadata = path.lstat()
    assert stat.S_ISDIR(metadata.st_mode), 'collaboration Skill parent must be a real directory'
    return (metadata.st_dev, metadata.st_ino, metadata.st_uid, stat.S_IMODE(metadata.st_mode))


def prepare_collaboration_target(product):
    """Own only a newly created empty leaf; the product must install the real Skill."""
    directory = Path(product.env['QODER_CONFIG_DIR']) / 'skills/hiroute-collaboration'
    try:
        directory.mkdir(mode=0o700)
    except FileExistsError:
        # Existing directories belong to the user. Do not normalize permissions;
        # the real settings operation must enforce its existing target guard.
        directory_identity(directory)
    else:
        product.qoder_owned_collaboration_directory = (directory, directory_identity(directory))
    return directory / 'SKILL.md'


def cleanup_context(product):
    fixture = getattr(product, 'qoder_fixture', None)
    owned = getattr(product, 'qoder_owned_files', {})
    collaboration_directory = getattr(product, 'qoder_owned_collaboration_directory', None)
    if not owned and not collaboration_directory:
        return
    if fixture:
        assert_settings_preserved(fixture)
    config = Path(product.env['QODER_CONFIG_DIR'] if not fixture else fixture['config'])
    # Never remove native history, auth, existing parent directories, or a changed
    # file. Project receipts are already inside Product's private owned root.
    parents = set()
    for name, expected in owned.items():
        path = Path(name)
        if not path.is_relative_to(config):
            continue
        assert not path.is_symlink() and path.is_file() and digest(path) == expected, \
            'refusing cleanup of replaced Qoder fixture material'
        path.unlink()
        if path.parent.parent == config / 'skills':
            parents.add(path.parent)
    for parent in parents:
        parent.rmdir()
    if collaboration_directory:
        directory, expected = collaboration_directory
        if directory.exists() or directory.is_symlink():
            assert directory_identity(directory) == expected and not any(directory.iterdir()), \
                'refusing cleanup of changed or nonempty collaboration Skill directory'
            directory.rmdir()


def task_session_root(fixture):
    """Find one task only inside this Product's HiRoute-owned metadata."""
    task_id = fixture['task_id']
    sessions = Path(fixture['product_storage']) / 'delegation-workers/sessions'
    found = []
    for marker in sessions.glob('*/.hiroute-native-root-v1.json'):
        assert not marker.is_symlink() and not marker.parent.is_symlink()
        value = json.loads(marker.read_text())
        if value['task_id'] != task_id:
            continue
        assert value['harness'] == 'qoder_cli'
        descriptor = json.loads((marker.parent / 'borrowed-native-context.json').read_text())
        assert descriptor['version'] == 1 and descriptor['harness'] == 'qoder_cli'
        assert descriptor['context'] == {'home': fixture['home'], 'config_root': fixture['config'],
                                         'workspace': fixture['project']}
        found.append(marker.parent)
    assert len(found) == 1, 'expected one exact HiRoute task-owned native root'
    return found[0]


def task_binding(fixture):
    binding = json.loads((task_session_root(fixture) / 'native-session-binding.json').read_text())
    assert binding['version'] == 1
    return binding['native_session_id']


def history_directory(fixture):
    """Pinned 1.1.65 layout for a short, fixture-owned canonical workspace only."""
    workspace = str(Path(fixture['project']).resolve(strict=True))
    assert len(workspace) <= 200, 'Qoder history fixture requires a short private workspace'
    return Path(fixture['config']) / 'projects' / re.sub('[^a-zA-Z0-9]', '-', workspace)


def exact_history(fixture):
    return history_for_session(fixture, task_binding(fixture))


def history_for_session(fixture, native_id):
    assert re.fullmatch(r'[a-zA-Z0-9_-]+', native_id), 'unsafe native fixture session ID'
    path = history_directory(fixture) / (native_id + '.jsonl')
    assert not path.is_symlink() and path.is_file(), 'exact Qoder fixture transcript is absent'
    assert fixture['receipt'] in path.read_text(), 'exact Qoder transcript lacks this task receipt'
    return str(path), native_id


def cancelled_history(fixture, run_id):
    """Read only this fixture's exact cancelled task; this is not Continue eligibility.

    Current runtime record_json is a registered test-only storage fact. Public
    status exposes observation session links, not the opaque native session ID.
    """
    task_session_root(fixture)  # Verify the task-owned descriptor and borrowed context.
    database = Path(fixture['product_storage']) / 'live/runtime.db'
    assert not database.is_symlink() and database.is_file()
    with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
        task_row = connection.execute(
            'SELECT record_json FROM delegation_tasks WHERE workspace_id=? AND task_id=?',
            ('personal/default', fixture['task_id'])).fetchone()
        run_row = connection.execute(
            'SELECT record_json FROM delegation_runs WHERE workspace_id=? AND run_id=?',
            ('personal/default', run_id)).fetchone()
    assert task_row is not None and run_row is not None, 'exact cancelled task/run record is missing'
    task, run = json.loads(task_row[0]), json.loads(run_row[0])
    assert (task['task_id'] == run['task_id'] == fixture['task_id']
            and task['latest_run_id'] == run['run_id'] == run_id
            and task['plan']['harness'] == 'qoder_cli'
            and run['progress']['state'] == 'cancelled'
            and run['progress']['cleanup'] == 'complete'), 'exact cancelled task identity or cleanup mismatch'
    session = task['session']
    assert (session and isinstance(session.get('native_session_id'), str)
            and session['native_session_id'] == session['acp_session_id']), \
        'cancelled Qoder task lacks one exact native session identity'
    return history_for_session(fixture, session['native_session_id'])


def assert_restricted_modes_unavailable(product, upstream):
    """Unsupported native policies must fail before any real model/tool action."""
    from delegation_product import worker_cli, wait_for_worker_result
    events = upstream.controls / 'native-context-events.jsonl'
    before = events.read_bytes() if events.exists() else b''
    root = product.root / 'qoder-restricted-workspace'
    root.mkdir(mode=0o700)
    marker = root / 'must-not-exist'
    for policy in ('approve-reads', 'deny-all'):
        command = ('worker exec --plan ' + product.plan_id + ' --cwd ' + str(root) +
                   ' --permission-policy ' + policy + ' --run-timeout 30 --no-wait' +
                   ' --submission-key qoder-restricted-' + policy + ' --file - --output json')
        status, response = worker_cli(product, command, 'Write must-not-exist then return done.', success=False)
        if status == 0:
            accepted = response['data']
            result = wait_for_worker_result(product, accepted['run_id'], expected='failed', timeout=30)
            assert not result.get('result'), 'unsupported Qoder policy returned a result'
        else:
            assert response.get('error', {}).get('code') == 'CAPABILITY_UNAVAILABLE', response
        assert not marker.exists(), 'unsupported Qoder policy executed a tool'
        assert (events.read_bytes() if events.exists() else b'') == before, \
            'unsupported Qoder policy sent a model prompt'
    return ['approve-reads', 'deny-all']


def replace_current_context(product):
    """Continue must use its original descriptor, not this new unlogged-in context."""
    home = product.root / 'replacement-qoder-home'
    home.mkdir(mode=0o700)
    config = home / '.qoder'
    config.mkdir(mode=0o700)
    product.env.update(HOME=str(home), QODER_CONFIG_DIR=str(config))


def project_conflict_settings(endpoint):
    """A native core-route override targeting an observable foreign loopback source."""
    selected = 'fixture-foreign/wrong-project-model'
    return {
        'general': {'enableAutoUpdate': False, 'sessionRetention': {'enabled': False},
                    'plan': {'modelRouting': False}},
        'disableAllHooks': True, 'promptSuggestionEnabled': False,
        'model': {'name': selected},
        'modelConfigs': {'overrides': [
            {'match': {'overrideScope': 'core'}, 'modelConfig': {'model': selected}}]},
        'providers': {'fixture-foreign': {'protocol': 'openai-responses', 'baseUrl': endpoint,
            'apiKey': 'synthetic-foreign-no-authority', 'model': 'wrong-project-model',
            'models': [{'model': 'wrong-project-model', 'capabilities': {'tools': True}}],
            'routing': {purpose: 'wrong-project-model' for purpose in (
                'utility', 'session_title', 'summary', 'compact', 'subagent')}}},
    }


def install_project_conflict(fixture, upstream):
    assert upstream.proxy_trap is None
    trap = NativeProxyTrap(upstream.controls / 'qoder-foreign-project-requests.log')
    upstream.proxy_trap = trap
    fixture['foreign_route_events'] = str(trap.path)
    settings = Path(fixture['project']) / '.qoder/settings.json'
    write_new(settings, json.dumps(project_conflict_settings(trap.url + '/v1')))
    protect_configuration(fixture, [settings])
