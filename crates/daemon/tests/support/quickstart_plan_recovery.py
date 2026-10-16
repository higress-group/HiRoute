"""Saved public Plan read -> actual candidate change -> complete revision-checked restore."""
import copy
import json
import subprocess


def run(product, plan, alternate_binding, submit):
    source_calls = len(product.upstream.requests)
    options = product.cli('routing', 'options', '--request-stdin',
                          payload={'authoring': {'plan_id': plan}})['data']['authoring']
    editor = options['editor']
    editor['smart']['economy'][0]['reasoning'] = {'kind': 'profile', 'profile': 'max'}
    editor['smart']['primary'][0]['reasoning'] = {'kind': 'profile', 'profile': 'max'}
    editor['smart']['complex_keywords'] = ['architecture']
    editor['smart']['judgment']['degree']['simple_threshold_millis'] = 650
    editor['smart']['judgment']['competence']['floor_millis'] = 700
    editor['limits'].update(context_window_tokens=16384, maximum_attempts=2,
                            request_timeout_ms=54000, attempt_timeout_ms=24000)
    request = {'change': {'schema': 'hiroute.plan-content-change/v2',
        'target': {'intent': 'update', 'plan_id': plan,
                   'expected_head_revision': options['expected_head_revision']},
        'editor': editor, 'consumed_draft': None}}
    custom = product.project / 'recovery-custom.json'
    prepared = product.cli('routing', 'preview', '--request-stdin',
                           '--apply-file', str(custom), payload=request)['data']
    submit(product, prepared)

    baseline = product.cli('routing', 'show', plan)
    saved = product.project / 'plan-before.json'
    with saved.open('x') as stream:
        saved.chmod(0o600)
        json.dump(baseline, stream)
    desired = baseline['data']['desired']
    change = product.cli('routing', 'preview', '--plan', plan, '--economy', alternate_binding,
                         '--apply-file', str(product.project / 'recovery-change.json'))['data']
    changed_editor = json.loads((product.project / 'recovery-change.json').read_text())['change']['editor']
    assert changed_editor['smart']['economy'][0]['reasoning']['profile'] == 'low'
    submit(product, change)
    changed = product.cli('routing', 'show', plan)['data']
    assert changed['desired']['strategy']['economy'][0]['binding_id'] == alternate_binding
    assert changed['desired'] != desired

    invalid = []
    for label, mutate in [
        ('failed', lambda v: v.update(status='unavailable')),
        ('other-plan', lambda v: v['data'].update(agent_plan_id='plan/another')),
        ('other-workspace', lambda v: v['data']['head']['reference'].update(workspace_id='personal/another')),
        ('unknown-field', lambda v: v['data']['desired'].update(unexpected='forbidden')),
        ('no-data', lambda v: v.pop('data')),
    ]:
        value = copy.deepcopy(baseline)
        mutate(value)
        invalid.append((label, json.dumps(value)))
    invalid += [('empty', ''), ('duplicate', '{"status":"succeeded","status":"succeeded"}'),
                ('trailing', json.dumps(baseline) + '{}')]
    for label, value in invalid:
        path = product.project / ('recovery-bad-' + label + '.json')
        path.write_text(value)
        output = product.project / ('recovery-blocked-' + label + '.json')
        product.cli('routing', 'preview', '--plan', plan, '--restore-from', str(path),
                    '--apply-file', str(output), expected=2)
        assert not output.exists()
    for extra in [('--name', 'Mixed'), ('--economy', alternate_binding), ('--decision', 'local_rules'),
                  ('--request-file', str(custom))]:
        product.cli('routing', 'preview', '--plan', plan, '--restore-from', str(saved), *extra,
                    '--apply-file', str(product.project / 'recovery-mixed.json'), expected=2)
    assert product.cli('routing', 'show', plan)['data'] == changed

    missing = copy.deepcopy(baseline)
    missing['data']['desired']['strategy']['economy'][0]['binding_id'] = 'binding/not-present'
    missing_file = product.project / 'recovery-missing-reference.json'
    missing_file.write_text(json.dumps(missing))
    missing_apply = product.project / 'recovery-missing-apply.json'
    rejected = subprocess.run([str(product.cli_path), 'routing', 'preview', '--plan', plan,
        '--restore-from', str(missing_file), '--apply-file', str(missing_apply), '--output', 'json'],
        env=product.env, cwd=product.project, capture_output=True, timeout=30)
    product.outputs.extend((rejected.stdout, rejected.stderr))
    assert rejected.returncode != 0 and not missing_apply.exists(), 'missing binding was substituted'

    args = ('routing', 'preview', '--plan', plan, '--restore-from', str(saved))
    restore = product.cli(*args, '--apply-file', str(product.project / 'recovery-stale.json'))['data']
    assert restore['preview']['plan_version']['configuration'] == desired
    assert product.cli('routing', 'show', plan)['data'] == changed, 'restore Preview changed saved state'
    concurrent = product.cli('routing', 'preview', '--plan', plan, '--name', 'Concurrent rename',
                            '--apply-file', str(product.project / 'recovery-concurrent.json'))['data']
    submit(product, concurrent)
    product.cli(*restore['submission']['arguments'], expected=3)
    fresh = product.cli(*args, '--apply-file', str(product.project / 'recovery-apply.json'))['data']
    submit(product, fresh)
    restored = product.cli('routing', 'show', plan)['data']
    assert restored['desired'] == desired, 'complete effective configuration was not recovered'
    assert restored['model_alias'] == baseline['data']['model_alias']
    assert restored['head']['head_revision'] > changed['head']['head_revision']
    assert len(product.upstream.requests) == source_calls, 'recovery made an implicit model call'
    assert len(product.cli('routing', 'list')['data']['plans']) == 1
    print(json.dumps({'scenario': 'installed-complete-plan-recovery', 'state': 'green',
                      'complete_effective_configuration': True, 'candidate_replacement': True,
                      'stale_and_exact_replay': True}), flush=True)
