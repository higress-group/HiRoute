"""V2 plans through real hiroute/hirouted and protected launcher, with read-only DB assertions."""
import json
import sqlite3
import sys
from publication_product import Product
from publication_process import bootstrap


def read_content(product):
    with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=ro', uri=True) as db:
        head = json.loads(db.execute('SELECT head_json FROM plan_heads WHERE plan_id=?',
                                    (product.plan_id,)).fetchone()[0])
        versions = [json.loads(row[0]) for row in db.execute(
            "SELECT version_json FROM plan_versions WHERE plan_id=? AND state='published' ORDER BY content_revision",
            (product.plan_id,))]
    return head, versions


def operation_count(product):
    with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=ro', uri=True) as db:
        return db.execute('SELECT COUNT(*) FROM operations').fetchone()[0]


def scenario(repository, boundary):
    product = Product(repository)
    try:
        bootstrap(product)
        assert product.model_alias == 'hiroute-richangbianma', product.model_alias
        catalog, _ = product.catalog()
        assert [row['id'] for row in catalog['data']] == [product.model_alias]
        head, versions = read_content(product)
        assert head['head_revision'] == 1 and len(versions) == 1
        old = versions[0]
        product.stop()
        product.start(boundary)
        editor = dict(product.editor, display_name='资料整理', purpose='Updated complete purpose',
                      limits=dict(product.editor['limits'], context_window_tokens=16384),
                      work={'harness': 'claude_code', 'protocol': 'messages'})
        change = {'schema': 'hiroute.plan-content-change/v2',
                  'target': {'intent': 'update', 'plan_id': product.plan_id, 'expected_head_revision': 1},
                  'editor': editor, 'consumed_draft': None}
        operations_before = operation_count(product)
        if boundary is None:
            for invalid in [0, 1.5, 9223372036854775807]:
                rejected_change = json.loads(json.dumps(change))
                rejected_change['editor']['limits']['context_window_tokens'] = invalid
                code, rejected = product.cli('routing preview', {'change': rejected_change}, success=False)
                assert code != 0, rejected
            assert operation_count(product) == operations_before
            assert read_content(product) == (head, versions)
        preview = product.preview('routing preview', {'change': change})
        assert operation_count(product) == operations_before, 'Preview created an Operation'
        assert read_content(product) == (head, versions), 'Preview published content'
        assert product.catalog()[0] == catalog, 'Preview changed the installed catalog'
        assert preview['plan_head']['model_alias'] == product.model_alias
        assert preview['plan_version']['configuration']['work'] == editor['work']
        assert preview['plan_version']['compiled']['body']['materialized']['attempt_owned']['limits']['context_window_tokens'] == 16384
        result, body, capability = product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                                                {'change': change}, 'content-two', crash=bool(boundary))
        if boundary:
            product.stop(crash=True)
            product.start()
            result = product.cli('routing apply', body, capability)[1]
        replay = product.cli('routing apply', body, capability)[1]
        assert replay['data'] == result['data'] and replay['data']['state'] == 'succeeded'
        assert operation_count(product) == operations_before + 1, 'recovery or retry duplicated the Operation'
        head, versions = read_content(product)
        assert head['head_revision'] == 2 and len(versions) == 2
        assert versions[0] == old, 'old full content changed'
        assert versions[1] == preview['plan_version'], 'installed content differs from confirmation'
        assert head == preview['plan_head']
        catalog, _ = product.catalog()
        assert [row['id'] for row in catalog['data']] == [product.model_alias]
        product.stop()
        product.start()
        assert read_content(product) == (head, versions)
        print(json.dumps({'scenario': 'plan-content-' + (boundary or 'publish'), 'state': 'green',
                          'cli_exit': 0, 'daemon_fault_exit': 86 if boundary else None}), flush=True)
    finally:
        product.close()


def draft_scenario(repository):
    product = Product(repository)
    try:
        bootstrap(product)
        before = read_content(product)
        before_models = product.catalog()[0]
        editor = dict(product.editor, display_name='尚未完成的草稿',
                      limits=dict(product.editor['limits'], context_window_tokens=32768), requirements={
            'tool': False, 'vision': False, 'streaming': False,
            'minimum_context_tokens': 0, 'minimum_output_tokens': 0})
        draft = {'schema': 'hiroute.plan-draft/v1', 'workspace_id': 'personal/default',
                 'draft_id': 'draft/editor-one', 'revision': 1, 'editor': editor}
        change = {'schema': 'hiroute.plan-draft-change/v1', 'workspace_id': 'personal/default',
                  'draft_id': draft['draft_id'], 'expected_revision': None,
                  'action': {'kind': 'save', 'draft': draft}}
        preview = product.preview('routing preview', {'change': change})
        assert preview['before'] is None and preview['after'] == draft, (preview, draft)
        assert product.cli('routing list')[1]['data']['drafts'] == []
        result, body, capability = product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                                                {'change': change}, 'draft-save')
        assert product.cli('routing apply', body, capability)[1]['data'] == result['data']
        product.stop()
        product.start()
        assert product.cli('routing list')[1]['data']['drafts'] == [draft]
        first_page = product.cli('routing list', {'limit': 1})[1]['data']
        assert len(first_page['plans']) == 1 and not first_page['drafts']
        page_query = {'limit': 1, 'cursor': first_page['next_cursor']}
        second_page = product.cli('routing list', page_query)[1]['data']
        assert not second_page['plans'] and second_page['drafts'] == [draft]
        assert second_page.get('next_cursor') is None
        assert read_content(product) == before
        assert product.catalog()[0] == before_models
        stale = product.cli('routing preview', {'change': change}, success=False)[1]
        assert stale['error']['code'] == 'CHANGE_PREVIEW_STALE', stale
        discard = dict(change, expected_revision=1, action={'kind': 'discard'})
        preview = product.preview('routing preview', {'change': discard})
        assert preview['before'] == draft and preview['after'] is None
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': discard}, 'draft-discard')
        assert product.cli('routing list')[1]['data']['drafts'] == []
        stale_page = product.cli('routing list', page_query, success=False)[1]
        assert stale_page['error']['code'] == 'CHANGE_PREVIEW_STALE'
        assert read_content(product) == before
        print(json.dumps({'scenario': 'plan-draft-cas-restart-no-publication', 'state': 'green', 'cli_exit': 0}), flush=True)
    finally:
        product.close()


def reject_legacy_writes(repository):
    product = Product(repository)
    try:
        bootstrap(product)
        before = product.cli('routing list')[1]['data']
        content_before = read_content(product)
        def operations():
            with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=ro', uri=True) as db:
                return db.execute('SELECT COUNT(*) FROM operations').fetchone()[0]
        count_before = operations()
        editor = product.editor
        desired = {'schema': 'hiroute.agent-plan-desired/v1',
                   'display_name': 'Legacy overwrite', 'purpose': editor['purpose'],
                   'requirements': editor['requirements'], 'limits': editor['limits'],
                   'strategy': {'mode': 'custom', 'candidates': editor['candidates']}}
        accepted = product.preview('routing preview', {'change': {
            'schema': 'hiroute.plan-content-change/v2', 'target': {'intent': 'create', 'creation_key': 'only-a-preview'},
            'editor': editor, 'consumed_draft': None}})
        for index, target in enumerate([{'intent': 'create'}, {'intent': 'update', 'agent_plan_id': product.plan_id, 'expected_revision': 1}]):
            change = {'schema': 'hiroute.routing-control-change/v1', 'target': target, 'desired': desired}
            for command in ['routing preview', 'routing apply']:
                body = {'change': change}
                if command.endswith('apply'):
                    body.update(accept_digest=accepted['change_digest'], expected_revisions=accepted['expected_revisions'], idempotency_key='reject-legacy-' + str(index))
                code, rejected = product.cli(command, body, success=False)
                assert code != 0 and rejected['error']['code'] == 'SCHEMA_INCOMPATIBLE', rejected
        assert operations() == count_before
        assert product.cli('routing list')[1]['data'] == before
        assert read_content(product) == content_before
        print(json.dumps({'scenario': 'legacy-create-update-preview-apply-rejected', 'state': 'green',
                          'rejected_at': 'local_control_application', 'rejected_cli_exit': 2,
                          'new_operations': 0}), flush=True)
    finally:
        product.close()


def invalid_confirmation_does_not_create_operation(repository):
    product = Product(repository)
    try:
        bootstrap(product)
        head, versions = read_content(product)
        change = {'schema': 'hiroute.plan-content-change/v2',
                  'target': {'intent': 'update', 'plan_id': product.plan_id,
                             'expected_head_revision': head['head_revision']},
                  'editor': dict(product.editor, purpose='Exact authorized edit'),
                  'consumed_draft': None}
        preview = product.preview('routing preview', {'change': change})
        count = operation_count(product)
        body = {'change': change, 'accept_digest': preview['change_digest'],
                'expected_revisions': preview['expected_revisions'],
                'idempotency_key': 'invalid-then-valid'}
        damaged = json.loads(json.dumps(body))
        damaged['change']['editor']['purpose'] = 'Unconfirmed edit'
        code, rejected = product.cli('routing apply', damaged, success=False)
        assert code != 0 and rejected['error']['code'] == 'CHANGE_PREVIEW_STALE', rejected
        assert operation_count(product) == count
        assert read_content(product) == (head, versions)
        accepted = product.cli('routing apply', body)[1]
        assert accepted['data']['state'] == 'succeeded'
        assert product.cli('routing apply', body)[1]['data'] == accepted['data']
        assert operation_count(product) == count + 1
        assert read_content(product)[1][-1] == preview['plan_version']
        print(json.dumps({'scenario': 'invalid-confirmation-keeps-idempotency-key-and-replay',
                          'state': 'green', 'new_operations': 1}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    invalid_confirmation_does_not_create_operation(sys.argv[1])
    for boundary in (None, 'before_install', 'after_target', 'after_gateway_durable', 'after_terminal'):
        scenario(sys.argv[1], boundary)

    draft_scenario(sys.argv[1])

    reject_legacy_writes(sys.argv[1])
