"""Public CLI Plan delegation choices, publication, omission and complete Restore."""
import json


def run(product, plan, binding, submit):
    baseline = product.cli('routing', 'show', plan)
    source_calls = len(product.upstream.requests)
    saved = product.project / 'worker-plan-before.json'
    with saved.open('x') as stream:
        saved.chmod(0o600)
        json.dump(baseline, stream)
    desired = baseline['data']['desired']
    assert not desired['delegation_enabled'] and desired.get('work') is None
    invalid = [
        (('--delegation', 'enable'), ['--worker-harness', '--worker-protocol']),
        (('--worker-harness', 'codex_cli'), ['--worker-protocol']),
        (('--worker-protocol', 'responses'), ['--worker-harness']),
        (('--delegation', 'yes'), ['--delegation']),
        (('--worker-harness', 'unknown', '--worker-protocol', 'responses'), ['--worker-harness']),
        (('--worker-harness', 'codex_cli', '--worker-protocol', 'chat_completions'), ['--worker-protocol']),
    ]
    for index, (flags, required) in enumerate(invalid):
        path = product.project / f'worker-plan-invalid-{index}.json'
        reply = product.cli('routing', 'preview', '--plan', plan, *flags,
                            '--apply-file', str(path), expected=2)
        assert reply['data']['required_choices'] == required, reply
        assert not path.exists(), 'invalid choices produced a submission file'
    assert product.cli('routing', 'show', plan)['data'] == baseline['data']

    # A routing-only Plan can legitimately have no purpose. The new delegation
    # choices must surface --purpose before any exact Apply file is emitted.
    template_path = product.project / 'worker-plan-purpose-template.json'
    product.cli('routing', 'preview', '--plan', plan, '--name', 'Purpose setup',
                '--apply-file', str(template_path))
    change = json.loads(template_path.read_text())['change']
    change['editor']['purpose'] = ''
    empty = product.cli('routing', 'preview', '--request-stdin',
                        '--apply-file', str(product.project / 'worker-plan-empty-purpose.json'),
                        payload={'change': change})['data']
    submit(product, empty)
    empty_state = product.cli('routing', 'show', plan)['data']
    assert empty_state['desired']['purpose'] == ''
    missing_path = product.project / 'worker-plan-missing-purpose.json'
    missing = product.cli('routing', 'preview', '--plan', plan, '--delegation', 'enable',
                          '--worker-harness', 'codex_cli', '--worker-protocol', 'responses',
                          '--apply-file', str(missing_path), expected=2)
    assert missing['data']['required_choices'] == ['--purpose'], missing
    assert not missing_path.exists()
    assert product.cli('routing', 'show', plan)['data'] == empty_state
    complete = product.cli('routing', 'preview', '--plan', plan, '--delegation', 'enable',
                           '--worker-harness', 'codex_cli', '--worker-protocol', 'responses',
                           '--purpose', 'Delegate bounded work',
                           '--apply-file', str(product.project / 'worker-plan-with-purpose.json'))['data']
    submit(product, complete)
    assert product.cli('routing', 'show', plan)['data']['desired']['purpose'] == 'Delegate bounded work'
    disabled = product.cli('routing', 'preview', '--plan', plan, '--delegation', 'disable',
                           '--purpose', ' ',
                           '--apply-file', str(product.project / 'worker-plan-clear-purpose.json'))['data']
    submit(product, disabled)
    assert product.cli('routing', 'show', plan)['data']['desired']['purpose'] == ''
    missing_again = product.cli('routing', 'preview', '--plan', plan, '--delegation', 'enable', expected=2)
    assert missing_again['data']['required_choices'] == ['--purpose'], missing_again
    restored = product.cli('routing', 'preview', '--plan', plan, '--restore-from', str(saved),
                           '--apply-file', str(product.project / 'worker-plan-purpose-restore.json'))['data']
    submit(product, restored)
    baseline = product.cli('routing', 'show', plan)
    assert baseline['data']['desired'] == desired

    # A new Plan has explicit work without copying a complete editor template.
    path = product.project / 'worker-plan-new-preview.json'
    product.cli('routing', 'preview', '--name', 'Delegation preview', '--purpose', 'Bounded work',
                '--mode', 'fixed_model', '--model', binding, '--delegation', 'enable',
                '--worker-harness', 'qoder_cli', '--worker-protocol', 'responses',
                '--apply-file', str(path))
    created_editor = json.loads(path.read_text())['change']['editor']
    assert created_editor['delegation_enabled']
    assert created_editor['work'] == {'harness': 'qoder_cli', 'protocol': 'responses'}
    assert len(product.cli('routing', 'list')['data']['plans']) == 1, 'Preview created a Plan'

    path = product.project / 'worker-plan-enable.json'
    preview = product.cli('routing', 'preview', '--plan', plan, '--delegation', 'enable',
                          '--worker-harness', 'codex_cli', '--worker-protocol', 'responses',
                          '--apply-file', str(path))['data']
    assert product.cli('routing', 'show', plan)['data'] == baseline['data']
    submit(product, preview)
    enabled = product.cli('routing', 'show', plan)['data']
    expected = dict(desired, delegation_enabled=True,
                    work={'harness': 'codex_cli', 'protocol': 'responses'})
    assert enabled['desired'] == expected, 'enabling work changed unrelated Plan configuration'
    assert enabled['model_alias'] == baseline['data']['model_alias']

    # Ordinary edits and disabling keep the exact saved worker selection.
    for label, flags in [('rename', ('--name', 'Delegation renamed')),
                         ('disable', ('--delegation', 'disable')),
                         ('enable-existing', ('--delegation', 'enable'))]:
        path = product.project / f'worker-plan-{label}.json'
        preview = product.cli('routing', 'preview', '--plan', plan, *flags,
                              '--apply-file', str(path))['data']
        editor = json.loads(path.read_text())['change']['editor']
        assert editor['work'] == expected['work'], 'omitted work reset the saved selection'
        submit(product, preview)
        current = product.cli('routing', 'show', plan)['data']['desired']
        assert current['work'] == expected['work']
        assert current['delegation_enabled'] == (label != 'disable')
        assert current['strategy'] == desired['strategy']
        assert current['limits'] == desired['limits']

    restored = product.cli('routing', 'preview', '--plan', plan, '--restore-from', str(saved),
                           '--apply-file', str(product.project / 'worker-plan-restore.json'))['data']
    submit(product, restored)
    assert product.cli('routing', 'show', plan)['data']['desired'] == desired
    assert len(product.upstream.requests) == source_calls, 'Plan setup made an implicit model call'
    return {'scenario': 'installed-cli-worker-plan-setup', 'state': 'green',
            'explicit_choices': True, 'preview_did_not_publish': True,
            'omitted_work_preserved': True, 'complete_restore': True,
            'empty_purpose_required_before_delegation': True}
