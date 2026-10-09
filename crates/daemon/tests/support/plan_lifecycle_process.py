"""Plan lifecycle through the real CLI, durable publication and daemon restart."""
import json
import sys
from publication_product import Product
from publication_process import bootstrap, configure_model_settings_v2
from plan_content_process import read_content, operation_count


def run(repository):
    product = Product(repository)
    try:
        product.collaboration_only = True
        product.install_codex_fixture()
        bootstrap(product)
        head, versions = read_content(product)
        for status in ('disabled', 'enabled'):
            change = {'schema': 'hiroute.plan-lifecycle-change/v1',
                      'plan_id': product.plan_id,
                      'expected_head_revision': head['head_revision'], 'status': status}
            count = operation_count(product)
            preview = product.public_cli('routing preview', {'change': change})[1]['data']
            # No Agent grant is configured yet, so even an enabled Plan has no public alias.
            assert preview['no_new_calls'], preview
            assert not preview['has_agent_references'] and not preview['has_version_holds'], preview
            assert read_content(product) == (head, versions)
            body = {'change': change, 'accept_digest': preview['change_digest'],
                    'expected_revisions': preview['expected_revisions'], 'idempotency_key': status}
            result = product.public_cli('routing apply', body)[1]
            assert result['data']['state'] == 'succeeded', result
            assert product.public_cli('routing apply', body)[1]['data'] == result['data']
            assert operation_count(product) == count + 1
            updated, retained = read_content(product)
            assert updated['status'] == status and updated['head_revision'] == head['head_revision'] + 1
            assert updated['reference'] == head['reference'] and updated['model_alias'] == head['model_alias']
            assert retained == versions, 'head-only change rewrote immutable content'
            product.stop()
            product.start()
            assert read_content(product) == (updated, versions)
            head = updated
        configure_model_settings_v2(product, [product.plan_id], 'default-reference',
                                    agent_id='agent_codex_default')
        assert [row['id'] for row in product.catalog()[0]['data']] == [product.model_alias]
        for status in ('disabled', 'deleted'):
            change = {'schema': 'hiroute.plan-lifecycle-change/v1', 'plan_id': product.plan_id,
                      'expected_head_revision': head['head_revision'], 'status': status}
            code, rejected = product.public_cli('routing preview', {'change': change}, success=False)
            assert code != 0 and rejected['error']['code'] == 'INVALID_ARGUMENTS', rejected
            assert read_content(product) == (head, versions)
        settings = product.cli('agents connect status ' + product.agent_context_id)[1]['data']
        restore = {'schema_version': {'major': 2, 'minor': 0},
                   'context_id': product.agent_context_id,
                   'model': {'intent': 'restore', 'restore_point_ref': settings['restore_point_ref']},
                   'restore_native_model': 'gpt-5.4'}
        preview = product.public_cli('agents restore preview', {'spec': restore})[1]['data']
        assert preview['applicable'], preview
        body = {'spec': preview['spec'], 'accept_digest': preview['accept_digest'],
                'dependency_digest': preview['dependency_digest'],
                'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'release-model'}
        applied = product.public_cli('agents restore apply', body)[1]
        assert applied['data']['state'] == 'succeeded', applied
        preview = product.public_cli('routing preview', {'change': change})[1]['data']
        assert not preview['has_agent_references'], preview
        print(json.dumps({'scenario': 'plan-lifecycle-restart-references', 'state': 'green',
                          'lifecycle_changes': 2, 'blocked_referenced_changes': 2,
                          'restored_model_references': 0}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
