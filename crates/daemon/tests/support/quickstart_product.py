"""Installed CLI journey. Real stores/Operations; external clients/providers are fixtures."""
import json
import copy
from pathlib import Path
import subprocess
import sys

from standalone_headless_product import InstalledProduct, native_draft, NATIVE_TOKEN
from quickstart_fixture import support_local_skill_challenge
from quickstart_decision import run as decision_onboarding
from quickstart_plan_recovery import run as plan_recovery
from quickstart_analysis import run as management_analysis


def submit(product, prepared):
    submission = prepared['submission']
    path = Path(submission['apply_file'])
    exact = path.read_bytes()
    assert path.stat().st_mode & 0o777 == 0o600
    result = product.cli(*submission['arguments'])
    assert result['operation']['state'] == 'succeeded', result
    # Discard a successful reply, then find/replay; quickstart_recovery also crashes the daemon.
    found = product.cli('operations', 'find', '--request-stdin',
                        payload=submission['operation_lookup'])['data']
    assert found['digest_matches'] and found['operation']['operation_id'] == result['operation']['operation_id']
    replay = product.cli(*submission['arguments'])
    assert replay['operation']['operation_id'] == result['operation']['operation_id']
    assert path.read_bytes() == exact
    return result


def run(repository):
    product = InstalledProduct(repository)
    stage = 'install'
    try:
        product.install()
        support_local_skill_challenge(product)
        product.start()
        stage = 'read-only-navigation'
        before = product.cli('compute', 'list')['data']['sources']
        assert 'candidate' not in product.cli('compute', 'connection', 'options')['data']
        request = product.cli('quickstart', '--request-id', 'quickstart-read')
        assert request['request_id'] == 'quickstart-read'
        first = product.cli('quickstart')['data']
        assert '--context' in first['required_choices'] and not first['effects']
        assert len(first['targets']) > 1 and first['selected_context'] is None
        assert not product.upstream.requests, 'Navigation made an unrequested provider call'
        unavailable = product.cli('quickstart', '--context', 'missing-profile', expected=2)
        assert unavailable['data']['required_choices'] == ['exact_available_context']
        assert product.cli('compute', 'list')['data']['sources'] == before
        assert not product.cli('routing', 'list')['data']['plans']

        stage = 'prepare-source-exact-file'
        candidate_ref = 'candidate/native/headless-product'
        product.register_secret(candidate_ref, NATIVE_TOKEN)
        draft = native_draft(product.upstream)
        draft['models'][0]['capabilities']['native_reasoning']['value'] = {
            'kind': 'discrete', 'parameter': 'reasoning_effort',
            'profiles': ['low', 'max'], 'default_profile': 'max'}
        alternate = copy.deepcopy(draft['models'][0])
        alternate['upstream_model_id'] += '-alternate'
        alternate['display_name'] = 'Alternate recovery candidate'
        draft['models'].append(alternate)
        checked = product.cli('compute', 'connection', 'test', '--request-stdin', payload={
            'kind': 'native', 'request': {'draft': draft,
            'input_candidate': {'candidate_ref': candidate_ref, 'candidate_revision': 1}}})['data']
        candidate = checked['candidate']['candidate']
        source_args = ('compute', 'connection', 'preview', '--candidate', candidate['candidate_ref'],
                       '--candidate-revision', str(candidate['candidate_revision']), '--all-selectable')
        source_file = product.project / 'source-apply.json'
        exact_candidate = product.cli('compute', 'connection', 'options', '--request-stdin',
                                      payload={'candidate': candidate})['data']['candidate']
        assert exact_candidate['candidate'] == candidate
        invalid_candidate = dict(candidate, candidate_revision=candidate['candidate_revision'] + 1)
        product.cli('compute', 'connection', 'options', '--request-stdin',
                    payload={'candidate': invalid_candidate}, expected=3)
        prepared = product.cli(*source_args, '--apply-file', str(source_file))['data']
        assert not product.cli('compute', 'list')['data']['sources'], 'Preview saved a source'
        assert NATIVE_TOKEN not in source_file.read_text()
        submit(product, prepared)
        sources = product.cli('compute', 'list')['data']['sources']
        assert len(sources) == 1
        binding = next(m['binding_id'] for m in sources[0]['models']
                       if m['upstream_model_id'] == draft['models'][0]['upstream_model_id'])
        alternate_binding = next(m['binding_id'] for m in sources[0]['models']
                                 if m['upstream_model_id'] == alternate['upstream_model_id'])
        product.cli('protected-input', 'release', '--candidate', candidate_ref)

        stage = 'direct-smart-route'
        route_file = product.project / 'route-apply.json'
        route_args = ('routing', 'preview', '--name', 'Quickstart economy', '--purpose', 'Routine and complex work',
                      '--mode', 'smart_saving', '--economy', binding, '--primary', binding)
        missing = product.cli(*route_args, expected=2)
        assert missing['data']['required_choices']
        unsupported = product.cli(*route_args, '--decision-service', 'ordinary-chat-model@1', expected=2)
        assert unsupported['data']['required_choices'] == ['--decision-service']
        for invalid_path in [product.project / 'missing-parent/apply.json', Path('/dev/null/apply.json')]:
            blocked = product.cli(*route_args, '--decision', 'local_rules', '--apply-file', str(invalid_path), expected=2)
            assert blocked['data']['required_choices'] == ['new_writable_private_apply_file']
            assert not invalid_path.exists()
        prepared = product.cli(*route_args, '--decision', 'local_rules', '--apply-file', str(route_file))['data']
        original = json.loads(route_file.read_text())
        editor = original['change']['editor']
        assert editor['limits']['maximum_attempts'] == 6 and editor['limits']['attempt_timeout_ms'] == 30000
        assert editor['smart']['classifier']['kind'] == 'local_rules'
        submit(product, prepared)
        plans = product.cli('routing', 'list')['data']['plans']
        assert len(plans) == 1, 'Quickstart created a disposable fixed Plan'
        plan = plans[0]['agent_plan_id']
        # Preview never overwrites a file, including an existing symlink.
        product.cli(*route_args, '--decision', 'local_rules', '--apply-file', str(route_file), expected=2)
        assert json.loads(route_file.read_text()) == original
        symlink = product.project / 'linked-apply.json'
        symlink.symlink_to(route_file)
        product.cli(*route_args, '--decision', 'local_rules', '--apply-file', str(symlink), expected=2)
        assert json.loads(route_file.read_text()) == original
        product.cli('routing', 'preview', '--request-file', str(route_file), '--name', 'Mixed',
                    '--apply-file', str(product.project / 'mixed.json'), expected=2)
        assert not (product.project / 'mixed.json').exists()

        stage = 'update-preserves-defaults-and-rejects-stale'
        update_file = product.project / 'update-apply.json'
        stale_file = product.project / 'stale-apply.json'
        update = product.cli('routing', 'preview', '--plan', plan, '--name', 'Renamed route',
                             '--apply-file', str(update_file))['data']
        stale = product.cli('routing', 'preview', '--plan', plan, '--purpose', 'Stale purpose',
                            '--apply-file', str(stale_file))['data']
        updated_editor = json.loads(update_file.read_text())['change']['editor']
        assert updated_editor['limits'] == editor['limits']
        assert updated_editor['smart'] == editor['smart']
        submit(product, update)
        product.cli(*stale['submission']['arguments'], expected=3)
        assert len(product.cli('routing', 'list')['data']['plans']) == 1

        stage = 'complete-plan-recovery'
        plan_recovery(product, plan, alternate_binding, submit)

        stage = 'connect-and-independent-management'
        codex = next(agent for agent in first['targets'] if agent['agent_id'] == 'agent_codex_default')
        context = codex['codex_access']['root_context_id']
        missing = product.cli('agents', 'connect', 'preview', '--context', context, '--plan', plan, expected=2)
        assert '--native-models' in missing['data']['required_choices']
        connect_file = product.project / 'connect-apply.json'
        blocked = product.cli('agents', 'connect', 'preview', '--context', context,
                              '--management', 'enable', '--apply-file', str(connect_file), expected=2)
        assert blocked['data']['required_choices'] == ['resolve_preview_blockers']
        assert not connect_file.exists()
        check = product.cli('agents', 'check', 'agent_codex_default', '--scope', 'collaboration')['data']
        assert check['skill_loading'] == 'proven', check
        prepared = product.cli('agents', 'connect', 'preview', '--context', context, '--plan', plan,
                               '--native-models', 'hiroute_only', '--native-default', 'plan',
                               '--management', 'enable', '--apply-file', str(connect_file))['data']
        submit(product, prepared)
        status = product.cli('agents', 'connect', 'status', '--effective', '--context', context, '--plan', plan)['data']
        assert status['state'] == 'configured' and status['management_skill']['state'] == 'configured'
        assert status['effective']['native_loading']['state'] == 'unknown'
        assert status['effective']['request_evidence']['state'] == 'pending'
        assert not status['effective']['observed_request']
        for _ in range(2):
            resumed = product.cli('quickstart', '--context', context, '--plan', plan)['data']
            assert resumed['stage'] == 'native_activation_or_verification_pending'
        assert len(product.cli('compute', 'list')['data']['sources']) == 1
        assert len(product.cli('routing', 'list')['data']['plans']) == 1

        stage = 'facet-only-owned-restore'
        restore_file = product.project / 'restore-management.json'
        restore = product.cli('agents', 'restore', 'preview', '--context', context,
                             '--facet', 'management_skill', '--apply-file', str(restore_file))['data']
        assert json.loads(restore_file.read_text())['spec']['model']['intent'] == 'keep'
        submit(product, restore)
        status = product.cli('agents', 'connect', 'status', context)['data']
        assert status['state'] == 'configured'
        assert status['management_skill']['state'] != 'configured'
        assert 'effective' not in status

        stage = 'decision-onboarding-and-routed-groups'
        decision_onboarding(product, plan, submit)

        stage = 'historical-management-analysis'
        management_analysis(product, plan)

        stage = 'terminal-output-and-exit'
        capture = product.project / 'capture'
        capture.mkdir(mode=0o700)
        script = '''"$1" routing preview --request-stdin < /dev/null > "$2/stdout.json" 2> "$2/stderr.txt"
code=$?
printf '%s\\n' "$code" > "$2/exit.txt"
exit "$code"
'''
        result = subprocess.run(['/bin/sh', '-c', script, 'capture', str(product.cli_path), str(capture)],
                                cwd=product.project, env=product.env, capture_output=True)
        assert result.returncode == 2 and (capture / 'exit.txt').read_text().strip() == '2'
        assert json.loads((capture / 'stdout.json').read_text())['error']['code'] == 'INVALID_ARGUMENTS'
        assert (capture / 'stderr.txt').read_text()
        print(json.dumps({'scenario': 'installed-cli-quickstart', 'state': 'green',
                          'native_loading': 'not_claimed', 'stories': ['US1', 'US2', 'US3', 'US4', 'US5', 'US6']}), flush=True)
    except Exception:
        print(json.dumps({'scenario': 'installed-cli-quickstart', 'state': 'red', 'stage': stage}), flush=True)
        raise
    finally:
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
