"""First real CLI/Client Core/role-all/installed Codex ACP/Gateway/result chain.

Only the external model upstream is deterministic. All HiRoute mutations use Preview/Apply
and the protected user-confirmation channel; no business SQL or replacement execution ports.
"""
import hashlib
import http.client
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from publication_product import Product, encoded
from publication_process import bootstrap, configure_model_settings_v2, plan_change


def current_revisions(product):
    change = plan_change(product, 'update', work=product.worker_work)
    return product.preview('routing preview', {'change': change})['expected_revisions']


def configure_worker_installation(product, harness, adapter, cli, node):
    """Select one real installation through the protected production management path."""
    discovered = product.control(
        'WorkerDependenciesDiscover', {'harness': harness})['data']
    revision = next(
        entry['revision'] for entry in discovered['selection_revisions']
        if entry['harness'] == harness)
    selection = {
        'harness': harness,
        'adapter_path': str(adapter),
        'cli_path': str(cli),
        'node_path': str(node),
        'expected_selection_revision': revision,
    }
    selected = product.control('SelectWorkerDependencies', selection)
    assert selected['operation']['state'] == 'succeeded', selected
    view = selected['data']
    assert next(
        entry['revision'] for entry in view['selection_revisions']
        if entry['harness'] == harness) == revision + 1, view
    assert {
        'harness': harness,
        'adapter_path': str(adapter),
        'cli_path': str(cli),
        'node_path': str(node),
    } in view['selected'], view


def select_real_main_codex(product, codex):
    """Keep the CPA fixture private, then select the installed native CLI for checks."""
    product.enable_cpa()
    private_codex = product.root / 'bin/codex'
    assert private_codex.is_file() and not private_codex.is_symlink()
    private_codex.unlink()
    private_codex.symlink_to(codex)


def configure_fixture_price(product):
    binding = product.binding
    rates = {
        'input_uncached': {'state': 'known', 'micros_per_million_tokens': 1_000_000},
        'output': {'state': 'known', 'micros_per_million_tokens': 2_000_000},
        'cache_read': {'state': 'known', 'micros_per_million_tokens': 0},
        'cache_write': {'state': 'known', 'micros_per_million_tokens': 0},
    }
    change = {
        'target_locator': {'kind': 'binding', 'binding_id': binding['binding_id']},
        'currency': 'USD', 'valuation_kind': 'usage_estimate',
        'action': {'kind': 'set', 'rates': rates},
        'expected_source_revision': binding['source_revision'],
        'expected_binding_revision': binding['revision'],
        'expected_override_revision': 0,
    }
    preview = product.preview('prices override preview', change)
    assert preview['after']['origin'] == 'manual', preview
    product.apply('prices override apply', 'ApplyPriceOverrideChange', preview,
                  {'spec': preview['spec']}, 'fixture-price')
    effective = product.preview('prices effective', {'targets': [{
        'query_id': 'fixture-price', 'target_locator': change['target_locator'],
        'currency': 'USD', 'valuation_kind': 'usage_estimate'}]})
    quote = effective['items'][0]['quote']
    assert quote['origin'] == 'manual' and quote['rates'] == rates, effective
    generation = effective['generation_ref']
    assert generation is not None and quote['generation_ref'] == generation, effective
    return generation


def request_as_configured_main_agent(product):
    catalog, _ = product.catalog()
    aliases = {entry['id'] for entry in catalog['data']}
    assert product.model_alias in aliases, catalog
    client = http.client.HTTPConnection('127.0.0.1', product.port, timeout=45)
    try:
        client.request('POST', '/v1/responses', body=encoded({
            'model': product.model_alias,
            'input': 'D-combined-main-agent request',
            'stream': False,
        }), headers={
            'X-HiRoute-Token': product.bearer(product.agent_connection),
            'Content-Type': 'application/json',
        })
        response = client.getresponse()
        body = response.read()
        product.outputs.append(body)
        assert response.status == 200 and b'fixture answer' in body, (response.status, body)
    finally:
        client.close()


def worker_cli(product, command, prompt=None, success=True):
    """Invoke the public Worker CLI without its removed JSON request-stdin surface."""
    args = [str(product.bin / 'hiroute'), *command.split()]
    result = subprocess.run(
        args, env=product.env, cwd=product.project,
        input=prompt.encode() if prompt is not None else None,
        capture_output=True, timeout=240)
    product.outputs.extend((result.stdout, result.stderr))
    envelope = json.loads(result.stdout)
    if success:
        assert result.returncode == 0, (command, envelope, result.stderr.decode(errors='replace'))
    return result.returncode, envelope


def worker_cli_text(product, command, success=True):
    args = [str(product.bin / 'hiroute'), *command.split()]
    result = subprocess.run(
        args, env=product.env, cwd=product.project,
        capture_output=True, timeout=240, text=True)
    product.outputs.extend((result.stdout.encode(), result.stderr.encode()))
    if success:
        assert result.returncode == 0, (command, result.stdout, result.stderr)
    return result.returncode, result.stdout, result.stderr


TERMINAL_RUN_STATES = ('succeeded', 'failed', 'cancelled', 'unknown')


def wait_for_worker_result(product, run_id, expected='succeeded', timeout=200):
    deadline = time.monotonic() + timeout
    while True:
        _, response = worker_cli(product, 'worker result --run ' + run_id + ' --output json')
        result = response['data']
        if result['run_state'] in TERMINAL_RUN_STATES:
            assert result['run_state'] == expected, result
            return result
        assert time.monotonic() < deadline, result
        time.sleep(.25)


def read_during_worker_turn(product, run_id, timeout=120):
    """Observe a persisted assistant batch while the real Worker turn is still open."""
    deadline = time.monotonic() + timeout
    while True:
        status_code, response = worker_cli(
            product, 'worker read --run ' + run_id + ' --max-bytes 16384 --output json',
            success=False)
        if status_code != 0:
            assert response.get('error', {}).get('code') == 'OBSERVATION_UNAVAILABLE', response
            _, status = worker_cli(
                product, 'worker status --run ' + run_id + ' --output json')
            assert status['data']['run_state'] not in TERMINAL_RUN_STATES, status
            assert time.monotonic() < deadline, response
            time.sleep(.25)
            continue
        read = response['data']
        if read['content_state'] == 'available' and read['text']:
            assert 'fixture progress one' in read['text'], read
            assert 'Host Database' not in read['text'], read
            _, status = worker_cli(
                product, 'worker status --run ' + run_id + ' --output json')
            assert status['data']['run_state'] not in TERMINAL_RUN_STATES, status
            assert read['run_state'] not in TERMINAL_RUN_STATES, read
            assert read['next_cursor'], read
            return {
                'cursor': read['next_cursor'],
                'state_revision': read['state_revision'],
                'segment': read['segment'],
                'window_start': read['window_start'],
                'window_end': read['window_end'],
                'bytes': len(read['text'].encode()),
            }
        assert time.monotonic() < deadline, read
        time.sleep(.25)


def run(repository, expected_sha):
    repo = Path(repository).resolve()
    actual_sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    assert actual_sha == expected_sha, 'candidate mismatch'
    product = Product(repo)
    product.enable_debug_diagnostics()
    tool_roundtrip = os.environ.get('HIROUTE_PRODUCT_TOOL_ROUNDTRIP') == '1'
    search_roundtrip = os.environ.get('HIROUTE_PRODUCT_SEARCH_ROUNDTRIP') == '1'
    lifecycle_roundtrip = os.environ.get('HIROUTE_PRODUCT_LIFECYCLE_ROUNDTRIP') == '1'
    progress_roundtrip = os.environ.get('HIROUTE_PRODUCT_PROGRESS_ROUNDTRIP') == '1'
    pricing_roundtrip = os.environ.get('HIROUTE_PRODUCT_PRICING_ROUNDTRIP') == '1'
    combined_roundtrip = os.environ.get('HIROUTE_PRODUCT_COMBINED_ROUNDTRIP') == '1'
    worker_harness = os.environ.get('HIROUTE_PRODUCT_WORKER_HARNESS', 'codex')
    assert worker_harness in ('codex', 'claude'), 'unsupported product Worker Harness'
    if search_roundtrip:
        assert worker_harness == 'codex', 'hosted search is supported only by the Responses Worker path'
    tool_roundtrip = tool_roundtrip or search_roundtrip
    scenario = 'B-real-tool-roundtrip' if tool_roundtrip else 'B-first-real-product-chain'
    if search_roundtrip:
        scenario = 'B-real-search-roundtrip'
    if lifecycle_roundtrip:
        scenario = 'A-real-worker-instance-lifecycle'
    if progress_roundtrip:
        scenario = 'A-real-worker-read-roundtrip'
    if pricing_roundtrip:
        scenario = 'C-real-pricing-observation'
    if combined_roundtrip:
        scenario = 'D-combined-model-and-collaboration'
    assert sum(bool(value) for value in (
        lifecycle_roundtrip, progress_roundtrip, tool_roundtrip,
        pricing_roundtrip, combined_roundtrip)) <= 1, \
        'lifecycle, progress, tool/search, pricing and combined fixtures are separate exact scenarios'
    stage = 'setup'
    try:
        codex = Path(os.environ['HIROUTE_WORKER_CODEX_BINARY']).resolve(strict=True)
        node = Path(os.environ['HIROUTE_WORKER_NODE']).resolve(strict=True)
        if worker_harness == 'codex':
            worker_binary = codex
            adapter = Path(os.environ['HIROUTE_WORKER_CODEX_ACP_ADAPTER']).resolve(strict=True)
            product.worker_work = {'harness': 'codex_cli', 'protocol': 'responses'}
        else:
            worker_binary = Path(os.environ['HIROUTE_WORKER_CLAUDE_BINARY']).resolve(strict=True)
            adapter = Path(os.environ['HIROUTE_WORKER_CLAUDE_ACP_ADAPTER']).resolve(strict=True)
            product.worker_work = {'harness': 'claude_code', 'protocol': 'messages'}
        product.startup_timeout = 210
        product.collaboration_only = True
        select_real_main_codex(product, codex)
        # The main Codex process is deliberately rooted at the isolated CPA fixture.
        # Keep the assertion below on that effective CODEX_HOME rather than on the
        # unused default beneath HOME.
        native = product.cpa_fixture
        product.codex_settings = native / 'config.toml'
        product.codex_settings.write_text(
            '# isolated first-chain main Agent configuration\n')
        product.codex_settings.chmod(0o600)
        original = product.codex_settings.read_bytes()
        if tool_roundtrip:
            (product.cpa_fixture / 'tool-roundtrip').touch()
        if search_roundtrip:
            (product.cpa_fixture / 'search-roundtrip').touch()
        if pricing_roundtrip:
            (product.cpa_fixture / 'pricing-roundtrip').touch()
        if progress_roundtrip:
            (product.cpa_fixture / 'progress-roundtrip').touch()
        stage = 'real-daemon-startup-and-source-plan-publication'
        bootstrap(product)
        stage = 'protected-worker-installation-selection'
        configure_worker_installation(
            product, product.worker_work['harness'], adapter, worker_binary, node)
        price_generation = configure_fixture_price(product) if pricing_roundtrip else None
        stage = 'publish-worker-plan'
        change = plan_change(
            product, 'update', delegation_enabled=True, work=product.worker_work)
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'worker-plan')
        stage = 'explicit-native-collaboration-capability-check'
        revisions = product.preview('routing preview', {'change': plan_change(product, 'update', work=product.worker_work)})['expected_revisions']
        check = {'agent_id': 'agent_codex_default', 'scope': 'collaboration', 'suite': 'quick', 'allow_model_call': False}
        consent = {'change_digest': 'sha256:' + hashlib.sha256(encoded(check)).hexdigest(), 'expected_revisions': revisions}
        capability = product.grant('CheckAgentConnection', consent, 'native-collaboration-check')
        _, checked = product.cli(
            'agents check agent_codex_default --scope collaboration', capability=capability)
        assert checked['data']['skill_loading'] == 'proven', checked
        assert checked['data']['trusted_cli_execution'] == 'proven', checked
        stage = 'confirmed-collaboration-with-local-worker-plan-policy'
        scan = product.preview('agents scan')
        context = next(agent['context_id'] for agent in scan['agents'] if agent['agent_id'] == 'agent_codex_default')
        settings = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': context,
                    'collaboration': {'intent': 'configure', 'settings': {
                        'trigger_mode': 'explicit'}}}
        preview = product.preview('agents connect preview', {'spec': settings})
        assert preview['applicable'], preview.get('blockers')
        assert preview['collaboration_effect'] == {
            'action': 'configure', 'trigger_mode': 'explicit'}, preview
        _, applied = product.cli('agents connect apply', {
            'spec': settings, 'accept_digest': preview['accept_digest'],
            'dependency_digest': preview['dependency_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'collaboration'})
        assert applied['data']['state'] == 'succeeded', applied
        assert (native / 'config.toml').read_bytes() == original
        collaboration_preserved_main_config = True
        main_model_routed = False
        if combined_roundtrip:
            stage = 'configure-main-model-alongside-collaboration'
            configure_model_settings_v2(
                product, [product.plan_id], 'combined-main-model',
                agent_id='agent_codex_default')
            stage = 'configured-main-agent-gateway-request'
            request_as_configured_main_agent(product)
            main_model_routed = True
        stage = 'same-uid-worker-plans-with-logical-agent'
        _, directory = worker_cli(product, 'worker plans --output json')
        plans = directory['data']['plans']
        assert len(plans) == 1, directory
        assert plans[0]['agent_plan_id'] == product.plan_id, plans
        assert plans[0]['purpose'] == product.editor['purpose'], plans
        assert plans[0]['harness'] == product.worker_work['harness'], plans
        assert plans[0]['protocol'] == product.worker_work['protocol'], plans
        assert plans[0]['availability'] == 'ready' and plans[0]['reason'] is None, plans
        restricted_policy = {}
        if worker_harness == 'codex':
            # codex-acp 1.10.0 advertises `read-only` but maps it to workspaceWrite. The
            # production path must fail closed before a prompt/upstream request instead of
            # claiming that approve-reads was enforced.
            stage = 'restricted-codex-policy-fails-before-prompt'
            restricted_workspace = product.root / 'restricted-workspace'
            restricted_workspace.mkdir()
            restricted_marker = restricted_workspace / 'must-not-exist'
            attempts_path = product.cpa_fixture / 'attempts.jsonl'
            before_attempts = len(attempts_path.read_text().splitlines()) \
                if attempts_path.exists() else 0
            restricted_command = (
                'worker exec --plan ' + product.plan_id
                + ' --cwd ' + str(restricted_workspace)
                + ' --permission-policy approve-reads'
                + ' --run-timeout 180 --no-wait'
                + ' --submission-key restricted-codex-start'
                + ' --file - --output json')
            _, restricted = worker_cli(
                product, restricted_command,
                'Write must-not-exist in the current directory, then reply done.')
            restricted = restricted['data']
            assert restricted['run_state'] == 'accepted', restricted
            deadline = time.monotonic() + 30
            while True:
                _, restricted_result = worker_cli(
                    product, 'worker result --run ' + restricted['run_id']
                    + ' --output json')
                restricted_result = restricted_result['data']
                if restricted_result['run_state'] in (
                        'succeeded', 'failed', 'cancelled', 'unknown'):
                    assert restricted_result['run_state'] == 'failed', restricted_result
                    break
                assert time.monotonic() < deadline, restricted_result
                time.sleep(.05)
            after_attempts = len(attempts_path.read_text().splitlines()) \
                if attempts_path.exists() else 0
            assert before_attempts == after_attempts
            assert not restricted_marker.exists()
            restricted_policy = {
                'policy': 'approve_reads',
                'run_state': restricted_result['run_state'],
                'prompt_reached_gateway': False,
                'workspace_mutated': False,
            }
        stage = 'formal-worker-exec'
        prompt = 'B-first-real: reply with the upstream answer; do not use tools.'
        duplicate_title = 'A-duplicate-title'
        parent_context = 'parent/' + ('x' * 249) if pricing_roundtrip else None
        if parent_context:
            assert len(parent_context) == 256
        if tool_roundtrip:
            prompt = 'B-first-real: perform the requested isolated plan update, then return the final answer.'
        if progress_roundtrip and worker_harness == 'claude':
            prompt = 'B-first-real: report progress, perform the isolated read, then return the final answer.'
        exec_command = (
            'worker exec --plan ' + product.plan_id
            + ' --cwd ' + str(product.project)
            + ' --run-timeout 180 --no-wait'
            + ' --submission-key first-real-start'
            + ((' --parent-task ' + parent_context) if parent_context else '')
            + ((' --title ' + duplicate_title) if lifecycle_roundtrip else '')
            + ' --file - --output json')
        observation_from_ms = int(time.time() * 1000) - 1000
        _, accepted = worker_cli(product, exec_command, prompt)
        accepted = accepted['data']
        assert accepted['submission_state'] == 'accepted', accepted
        assert accepted['run_state'] == 'accepted' and not accepted['replayed'], accepted
        status, injected = worker_cli(
            product, exec_command + ' --caller forged', prompt, success=False)
        assert status == 2 and injected['status'] == 'usage_error', injected
        progress = {}
        if progress_roundtrip:
            stage = 'real-worker-turn-internal-read'
            progress['exec'] = read_during_worker_turn(product, accepted['run_id'])
        stage = 'real-worker-gateway-and-persisted-result'
        result = wait_for_worker_result(product, accepted['run_id'])
        assert 'fixture answer' in result['result'], result
        if progress_roundtrip:
            _, tail = worker_cli(
                product, 'worker read --run ' + accepted['run_id']
                + ' --cursor ' + progress['exec']['cursor']
                + ' --max-bytes 16384 --output json')
            assert tail['data']['content_state'] == 'available', tail
            assert tail['data']['run_state'] == 'succeeded', tail
            assert 'Host Database' not in tail['data']['text'], tail
            _, text_read, text_error = worker_cli_text(
                product, 'worker read --run ' + accepted['run_id'] + ' --max-bytes 16384')
            assert accepted['run_id'] in text_read and 'fixture progress one' in text_read, \
                (text_read, text_error)
            assert 'Host Database' not in text_read, (text_read, text_error)
        lifecycle = {}
        if lifecycle_roundtrip:
            stage = 'formal-list-show-wait-result'
            _, shown = worker_cli(
                product, 'worker status --run ' + accepted['run_id']
                + ' --output json')
            assert shown['data']['run_id'] == accepted['run_id'], shown
            assert shown['data']['task_id'] == accepted['task_id'], shown
            _, shown_by_task = worker_cli(
                product, 'worker status --task ' + accepted['task_id']
                + ' --output json')
            assert shown_by_task['data']['run_id'] == accepted['run_id'], shown_by_task
            _, by_key = worker_cli(
                product, 'worker status --submission-key first-real-start'
                + ' --operation start --output json')
            assert by_key['data']['task_id'] == accepted['task_id'], by_key
            _, waited = worker_cli(
                product, 'worker wait --run ' + accepted['run_id']
                + ' --wait-timeout 1 --output json')
            assert waited['data']['run_state'] == 'succeeded', waited
            assert not waited['data']['timed_out'], waited

        if lifecycle_roundtrip or progress_roundtrip:
            stage = 'formal-exact-session-continue'
            continuation_prompt = (
                'B-continue-real: continue the existing task and reply with the upstream answer; '
                'do not use tools.')
            if progress_roundtrip and worker_harness == 'claude':
                continuation_prompt = (
                    'B-continue-real: report progress, perform the isolated read, then return '
                    'the final answer.')
            continuation_command = (
                'worker continue --task ' + accepted['task_id']
                + ' --expected-latest-run ' + accepted['run_id']
                + ' --run-timeout 180 --no-wait'
                + ' --submission-key first-real-continue'
                + ' --file - --output json')
            _, continued = worker_cli(product, continuation_command, continuation_prompt)
            continued = continued['data']
            assert continued['task_id'] == accepted['task_id'], continued
            assert continued['run_id'] != accepted['run_id'] and not continued['replayed'], continued
            if progress_roundtrip:
                stage = 'real-worker-continue-turn-internal-read'
                progress['continue'] = read_during_worker_turn(product, continued['run_id'])
            continued_result = wait_for_worker_result(product, continued['run_id'])
            assert 'fixture answer' in continued_result['result'], continued_result
            if progress_roundtrip:
                _, continued_tail = worker_cli(
                    product, 'worker read --run ' + continued['run_id']
                    + ' --cursor ' + progress['continue']['cursor']
                    + ' --max-bytes 16384 --output json')
                assert continued_tail['data']['content_state'] == 'available', continued_tail
                assert continued_tail['data']['run_state'] == 'succeeded', continued_tail
                assert 'Host Database' not in continued_tail['data']['text'], continued_tail
            _, continue_replay = worker_cli(
                product, continuation_command, continuation_prompt)
            assert continue_replay['data']['replayed'], continue_replay
            assert continue_replay['data']['run_id'] == continued['run_id'], continue_replay

        if lifecycle_roundtrip:
            stage = 'formal-durable-cancel'
            cancel_prompt = 'B-cancel-real hold-old-request until the run is cancelled.'
            cancel_exec_command = exec_command.replace(
                'first-real-start', 'first-real-cancel-start').replace(
                    ' --title ' + duplicate_title, ' --title A-cancel-title')
            _, held = worker_cli(product, cancel_exec_command, cancel_prompt)
            held = held['data']
            deadline = time.monotonic() + 120
            while not (product.cpa_fixture / 'held').exists():
                assert time.monotonic() < deadline, 'held upstream request did not start'
                time.sleep(.05)
            cancel_command = (
                'worker cancel --run ' + held['run_id']
                + ' --idempotency-key first-real-cancel --reason user-requested'
                + ' --output json')
            _, cancelled = worker_cli(product, cancel_command)
            cancelled = cancelled['data']
            assert cancelled['run_id'] == held['run_id'], cancelled
            assert cancelled['run_state'] in ('cancelling', 'cancelled'), cancelled
            (product.cpa_fixture / 'release').touch()
            deadline = time.monotonic() + 120
            while True:
                _, cancelled_result = worker_cli(
                    product, 'worker result --run ' + held['run_id']
                    + ' --output json')
                cancelled_result = cancelled_result['data']
                if cancelled_result['run_state'] == 'cancelled':
                    break
                assert time.monotonic() < deadline, cancelled_result
                time.sleep(.1)
            _, cancel_replay = worker_cli(product, cancel_command)
            assert cancel_replay['data']['run_id'] == cancelled['run_id'], cancel_replay
            assert cancel_replay['data']['run_state'] == 'cancelled', cancel_replay

            stage = 'formal-title-and-exact-list-pagination'
            second_command = exec_command.replace(
                'first-real-start', 'second-same-title-start')
            _, second = worker_cli(product, second_command, prompt)
            second = second['data']
            assert second['task_id'] != accepted['task_id'], second
            assert second['title'] == duplicate_title, second
            wait_for_worker_result(product, second['run_id'])

            automatic_prompt = (
                'A-auto-title\nB-worker: reply with the upstream answer; do not use tools.')
            automatic_command = (
                'worker exec --plan ' + product.plan_id
                + ' --cwd ' + str(product.project)
                + ' --run-timeout 180 --no-wait'
                + ' --submission-key automatic-title-start'
                + ' --file - --output json')
            _, automatic = worker_cli(product, automatic_command, automatic_prompt)
            automatic = automatic['data']
            assert automatic['title'] == 'A-auto-title', automatic
            wait_for_worker_result(product, automatic['run_id'])

            _, first_page = worker_cli(
                product, 'worker list --title ' + duplicate_title
                + ' --limit 1 --output json')
            first_page = first_page['data']
            assert len(first_page['tasks']) == 1 and first_page['next_cursor'], first_page
            _, second_page = worker_cli(
                product, 'worker list --title ' + duplicate_title
                + ' --limit 1 --cursor ' + first_page['next_cursor']
                + ' --output json')
            second_page = second_page['data']
            listed = first_page['tasks'] + second_page['tasks']
            assert len(listed) == 2 and not second_page['next_cursor'], listed
            assert {task['task_id'] for task in listed} == {
                accepted['task_id'], second['task_id']}, listed
            assert all(task['title'] == duplicate_title for task in listed), listed
            mismatch_status, mismatch = worker_cli(
                product, 'worker list --title A-auto-title --limit 1 --cursor '
                + first_page['next_cursor'] + ' --output json', success=False)
            assert mismatch_status != 0 and mismatch['status'] != 'succeeded', mismatch
            lifecycle = {
                'list_show_wait_result': True,
                'continued_same_task': True,
                'continue_replay_sent_no_second_prompt': True,
                'cancelled_state': cancelled_result['run_state'],
                'cancel_terminal_result_observed': True,
                'cancel_replay_same_run': True,
                'automatic_title': automatic['title'],
                'duplicate_title_tasks': len(listed),
                'filter_cursor_mismatch_rejected': True,
            }
        pricing_observation = {}
        if pricing_roundtrip:
            stage = 'formal-session-and-value-query'
            observation_to_ms = int(time.time() * 1000) + 60_000
            sessions_query = {
                'schema': 'hiroute.observation.query/v2',
                'intent': {'view': 'sessions', 'query': {
                    'from_ms': observation_from_ms, 'to_ms': observation_to_ms,
                    'session_id': None, 'request_id': None, 'limit': 50, 'cursor': None,
                    'agent_id': None, 'plan_id': product.plan_id, 'native_model': None,
                    'outcome': None, 'only_model_switch': False}},
            }
            value_query = {
                'schema': 'hiroute.observation.query/v2',
                'intent': {'view': 'value', 'query': {
                    'from_ms': observation_from_ms, 'to_ms': observation_to_ms,
                    'session_id': None, 'plan_id': product.plan_id, 'currency': 'USD'}},
            }
            revisions = current_revisions(product)
            sessions_cap = product.grant('ListSessionsV2', {
                'change_digest': 'sha256:' + hashlib.sha256(encoded(sessions_query)).hexdigest(),
                'expected_revisions': revisions}, 'observation-sessions')
            value_cap = product.grant('GetValueV2', {
                'change_digest': 'sha256:' + hashlib.sha256(encoded(value_query)).hexdigest(),
                'expected_revisions': revisions}, 'observation-value')
            deadline = time.monotonic() + 30
            while True:
                sessions = product.cli('sessions list', sessions_query, sessions_cap)[1]['data']
                if sessions['sessions']:
                    break
                assert time.monotonic() < deadline, {
                    'sessions': sessions, 'run_id': accepted['run_id']}
                time.sleep(.05)
            session_id = sessions['sessions'][0]['session_id']
            timeline_query = {
                'schema': 'hiroute.observation.query/v2',
                'intent': {'view': 'timeline', 'query': {
                    'from_ms': observation_from_ms, 'to_ms': observation_to_ms,
                    'session_id': session_id, 'request_id': None, 'limit': 50, 'cursor': None,
                    'agent_id': None, 'plan_id': product.plan_id, 'native_model': None,
                    'outcome': None, 'only_model_switch': False}},
            }
            timeline_cap = product.grant('GetSessionTimelineV2', {
                'change_digest': 'sha256:' + hashlib.sha256(encoded(timeline_query)).hexdigest(),
                'expected_revisions': revisions}, 'observation-timeline')
            while True:
                timeline = product.cli('sessions show', timeline_query, timeline_cap)[1]['data']
                values = product.cli('value show', value_query, value_cap)[1]['data']
                linked = [item for item in timeline['requests']
                          if item['run_id'] == accepted['run_id']]
                amount = next((item for item in values['amounts']
                               if item['currency'] == 'USD'
                               and item['valuation_kind'] == 'usage_estimate'), None)
                usage = {item['metric']: item for item in values['usage']}
                complete = (linked and amount is not None
                            and linked[0]['parent_context_ref'] == parent_context
                            and amount['known_sum_micros'] == 4
                            and amount['coverage'] == 'partial'
                            and amount['missing_contribution_count'] == 1
                            and values['pending_requests'] == 0
                            and values['provisional_requests'] == 0
                            and usage['input']['known_sum'] == 4
                            and usage['output']['known_sum'] == 2
                            and usage['cache_read']['known_sum'] == 0
                            and usage['cache_write']['known_sum'] is None
                            and usage['cache_write']['coverage'] == 'unknown'
                            and usage['cache_write']['missing_attempt_count'] == 1
                            and usage['reasoning']['known_sum'] == 0)
                if complete:
                    break
                assert time.monotonic() < deadline, {
                    'sessions': sessions, 'timeline': timeline,
                    'values': values, 'run_id': accepted['run_id']}
                time.sleep(.05)
            pricing_observation = {
                'manual_price_generation': price_generation['id'],
                'linked_run_id': linked[0]['run_id'],
                'session_id': linked[0]['session_id'],
                'request_id': linked[0]['request_id'],
                'parent_context_preserved': linked[0]['parent_context_ref'] == parent_context,
                'known_sum_micros': amount['known_sum_micros'],
                'coverage': amount['coverage'],
                'usage': {key: value['known_sum'] for key, value in usage.items()},
            }
        attempts = [json.loads(line) for line in (product.cpa_fixture / 'attempts.jsonl').read_text().splitlines()]
        expected_worker_requests = 2 if tool_roundtrip else 1
        if lifecycle_roundtrip:
            expected_worker_requests += 4
        if progress_roundtrip:
            expected_worker_requests += 1 if worker_harness == 'codex' else 3
        # Claude ACP may issue one additional upstream request during this lifecycle.
        # The completed task/result and bounded request count are the contract;
        # the adapter's internal turn count is not.
        allowed_worker_counts = {expected_worker_requests}
        if lifecycle_roundtrip and worker_harness == 'claude':
            allowed_worker_counts.add(expected_worker_requests + 1)
        worker_attempts = [attempt for attempt in attempts if attempt['delegated_goal']]
        main_attempts = [attempt for attempt in attempts if not attempt['delegated_goal']]
        assert len(worker_attempts) in allowed_worker_counts, attempts
        assert len(attempts) == len(worker_attempts) + int(combined_roundtrip), attempts
        assert all(attempt['stream'] for attempt in worker_attempts), attempts
        if combined_roundtrip:
            assert len(main_attempts) == 1 and not main_attempts[0]['stream'], attempts
        else:
            assert not main_attempts, attempts
        if tool_roundtrip:
            assert not attempts[0]['tool_output_valid'] and attempts[1]['tool_output_valid'], attempts
        if search_roundtrip:
            assert all(attempt['search_declared'] for attempt in attempts), attempts
            assert not attempts[0]['search_replayed'] and attempts[1]['search_replayed'], attempts
        stage = 'idempotent-replay'
        _, replay = worker_cli(product, exec_command, prompt)
        assert replay['data']['replayed'] and replay['data']['run_id'] == accepted['run_id'], replay
        assert len((product.cpa_fixture / 'attempts.jsonl').read_text().splitlines()) == len(attempts)
        main_config_unchanged = (native / 'config.toml').read_bytes() == original
        assert main_config_unchanged != combined_roundtrip
        product.stop()
        diagnostics = product.diagnostics_snapshot()
        assert diagnostics['state'] == 'complete', diagnostics
        assert diagnostics['level_applied'] == {
            'level': 'debug', 'revision': 0, 'source': 'smoke_override'}, diagnostics
        diagnostics.pop('path')
        print(json.dumps({'scenario': scenario, 'state': 'green', 'candidate': actual_sha,
            'cli_process_exit': 0, 'worker_state': result['run_state'],
            'result_readable': True, 'upstream_requests': len(attempts), 'replay_sent_no_second_prompt': True,
            'tool_roundtrip': tool_roundtrip,
            'search_roundtrip': search_roundtrip,
            'pricing_roundtrip': pricing_roundtrip,
            'pricing_observation': pricing_observation,
            'combined_roundtrip': combined_roundtrip,
            'main_model_routed': main_model_routed,
            'model_and_collaboration_coexist': combined_roundtrip,
            'lifecycle_roundtrip': lifecycle_roundtrip,
            'lifecycle': lifecycle,
            'progress_roundtrip': progress_roundtrip,
            'turn_internal_reads': progress,
            'local_same_uid_worker_plans_ready': True,
            'worker_harness': worker_harness,
            'restricted_policy': restricted_policy,
            'ordinary_cli_without_inherited_fd': True,
            'collaboration_preserved_main_config': collaboration_preserved_main_config,
            'main_config_unchanged': main_config_unchanged,
            'caller_field_rejected_at_cli': True,
            'diagnostics': diagnostics,
            'binaries': {name: hashlib.sha256((repo / 'target/debug' / name).read_bytes()).hexdigest() for name in ('hiroute', 'hirouted')},
            'harness_sha256': hashlib.sha256(worker_binary.read_bytes()).hexdigest()}), flush=True)
    except Exception:
        attempts_file = product.cpa_fixture / 'attempts.jsonl' if hasattr(product, 'cpa_fixture') else None
        attempts = [json.loads(line) for line in attempts_file.read_text().splitlines()] if attempts_file and attempts_file.exists() else []
        try:
            product.stop()
            daemon_stop_state = 'stopped'
        except Exception:
            daemon_stop_state = 'failed'
        evidence = repo / 'target/product-e2e-evidence' / (
            scenario + '-' + worker_harness + '-' + str(os.getpid()))
        diagnostics = product.preserve_diagnostics(evidence)
        print(json.dumps({'scenario': scenario, 'state': 'red', 'candidate': actual_sha,
            'stage': stage, 'upstream_attempts': attempts,
            'daemon_stop_state': daemon_stop_state, 'diagnostics': diagnostics}), flush=True)
        raise
    finally:
        product.close()
        for output in product.outputs:
            for line in output.decode(errors='replace').splitlines():
                if line.startswith('native collaboration check: '):
                    print(line, flush=True)


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
