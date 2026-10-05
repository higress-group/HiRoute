"""Pi-specific native resources and exact task transcript; reuse common product journeys."""
import json
from pathlib import Path
import shlex
from native_context_fixture import prepare_skill, write_new, protect_configuration, assert_preserved


def prepare_context(product, fixture):
    root = Path(fixture['config'])
    package = root / 'installed-receipt-package'
    skill = prepare_skill(package / 'skills', 'native-context-package', 'package')
    write_new(package / 'package.json', json.dumps({'name': 'hiroute-acceptance-resources',
        'version': '1.0.0', 'pi': {'skills': ['./skills']}}))
    fixture['skills'].append(skill)
    fixture['product_storage'] = str(product.storage)
    fixture['credential_helper_trap'] = str(root / 'credential-helper-must-not-run')
    write_new(root / 'auth.json', json.dumps({'anthropic': {'type': 'api_key',
        'key': '!printf leaked > ' + shlex.quote(fixture['credential_helper_trap'])},
        'hiroute-worker': {'type': 'api_key', 'key': 'saved-key-must-not-override-run'}}))
    write_new(root / 'models.json', json.dumps({'providers': {'ambient': {
        'baseUrl': 'http://127.0.0.1:9/v1', 'api': 'openai-responses', 'apiKey': 'wrong-native-key',
        'models': [{'id': 'wrong-native-model', 'contextWindow': 32768, 'maxTokens': 4096}]}}}))
    write_new(root / 'settings.json', json.dumps({'defaultProvider': 'ambient',
        'defaultModel': 'wrong-native-model', 'packages': [str(package)],
        'extensions': [str(root / 'missing-extension-must-not-be-installed')],
        'compaction': {'enabled': False}, 'retry': {'enabled': True}}))
    protect_configuration(fixture, [root / name for name in ('auth.json', 'models.json', 'settings.json')]
        + [package / 'package.json', Path(skill['skill']), Path(skill['script'])])


def history_directory(fixture):
    return Path(fixture['product_storage']) / 'delegation-workers/sessions'


def exact_history(fixture):
    found = []
    for marker in history_directory(fixture).glob('*/.hiroute-native-root-v1.json'):
        assert not marker.is_symlink() and not marker.parent.is_symlink()
        value = json.loads(marker.read_text())
        if value['task_id'] != fixture['task_id']:
            continue
        assert value['harness'] == 'pi'
        path = marker.parent / 'native-pi.jsonl'
        assert path.is_file() and not path.is_symlink()
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        assert rows[0]['type'] == 'session' and rows[0]['cwd'] == fixture['project']
        assert fixture['receipt'] in path.read_text(), 'Pi history lacks this task receipt'
        found.append((str(path), rows[0]['id']))
    assert len(found) == 1, 'expected one exact task-owned Pi transcript'
    assert not Path(fixture['credential_helper_trap']).exists(), 'Worker executed native credential helper'
    return found[0]


def corrupt_history_refuses_model_call(product, fixture, task, upstreams, variant="truncated-json"):
    from delegation_product import worker_cli, wait_for_worker_result
    path, identity = exact_history(fixture)
    path = Path(path)
    original = path.read_bytes()
    before = [upstream.request_count() for upstream in upstreams]
    try:
        # A correct header with truncated JSON body previously loaded as an empty session.
        if variant == 'truncated-json':
            damaged = original.split(b'\n', 1)[0] + b'\n{"type":"message",broken\n'
        else:
            rows = [json.loads(line) for line in original.splitlines()]
            if variant == 'missing-content':
                entry = next(row for row in rows[1:] if row.get('type') == 'message' and row['message']['role'] == 'user')
                del entry['message']['content']
            elif variant == 'old-session-version':
                rows[0]['version'] = 2
            elif variant == 'unknown-session-version':
                rows[0]['version'] = 4
            else:
                raise AssertionError('unknown native corruption fixture')
            damaged = ('\n'.join(json.dumps(row) for row in rows) + '\n').encode()
        path.write_bytes(damaged)
        corrupt = path.read_bytes()
        command = ('worker continue --task ' + task['task_id'] + ' --expected-latest-run '
            + task['run_id'] + ' --run-timeout 30 --no-wait --submission-key pi-corrupt-history-' + variant + ' --file - --output json')
        status, response = worker_cli(product, command, 'Continue the same task.', success=False)
        if status == 0:
            result = wait_for_worker_result(product, response['data']['run_id'], expected='failed', timeout=45)
            assert not result.get('result')
        else:
            assert response['error']['code'] == 'CAPABILITY_UNAVAILABLE', response
        assert [upstream.request_count() for upstream in upstreams] == before
        assert path.read_bytes() == corrupt, 'corrupt Pi transcript was silently repaired'
        assert_preserved(fixture)
        return {'variant':variant, 'native_session_id': identity, 'model_posts': 0, 'history_unchanged': True}
    finally:
        path.write_bytes(original)


def verify_corrupt_history_variants(product, fixture, task, upstreams):
    from delegation_product import worker_cli, wait_for_worker_result
    from native_context_product import wait_for_resumable_task
    results = [corrupt_history_refuses_model_call(product, fixture, task, upstreams)]
    for variant in ('missing-content', 'old-session-version', 'unknown-session-version'):
        # Give each corruption an independently healthy task. A previously refused
        # Continue cannot mask a later variant by withdrawing that task's capability.
        command = ('worker exec --plan ' + fixture['plan_id'] + ' --cwd ' + str(product.project)
            + ' --run-timeout 120 --no-wait --submission-key pi-history-' + variant + ' --file - --output json')
        prompt = fixture['boundary']['marker'] + ': use the native context Skills and receipt tool.'
        _, response = worker_cli(product, command, prompt)
        healthy = response['data']
        wait_for_worker_result(product, healthy['run_id'], timeout=130)
        wait_for_resumable_task(product, healthy['task_id'], healthy['run_id'])
        independent = dict(fixture, task_id=healthy['task_id'])
        results.append(corrupt_history_refuses_model_call(product, independent, healthy, upstreams, variant))
    return results
