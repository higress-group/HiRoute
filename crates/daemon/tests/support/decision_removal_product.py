"""Active-reference removal through the public CLI and real daemon, without provider calls."""
import copy
import json
import sqlite3
import sys

from publication_product import Product, V1
from publication_process import bootstrap, judgment_fixture, plan_change
from plan_content_process import read_content, operation_count


def run(repository):
    for deleted in (False, True):
        product = Product(repository)
        product.collaboration_only = True
        case = 'deleted' if deleted else 'historical'
        service_id = 'decision/removal-' + case

        def data(command, payload=None):
            return product.public_cli(command, payload)[1]['data']

        def apply_service(preview, key):
            request = {'schema_version': V1, 'spec': preview['normalized_spec'],
                       'accept_digest': preview['change_digest'],
                       'expected_revisions': preview['expected_revisions'], 'idempotency_key': key}
            result = data('decision connections apply', request)
            assert result['state'] == 'succeeded', result
            assert data('decision connections apply', request) == result
            return result

        def removal_preview():
            return data('decision removal preview', {'id': service_id, 'expected_revision': 1})

        def rows():
            with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=ro', uri=True) as db:
                return {table: db.execute(f'SELECT * FROM {table} ORDER BY 1,2,3').fetchall()
                        for table in ('plan_versions', 'plan_heads', 'decision_plan_references')}

        def source():
            with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=ro', uri=True) as db:
                return json.loads(db.execute('SELECT source_json FROM compute_management_sources WHERE source_id=?',
                                            (product.source_id,)).fetchone()[0])

        try:
            bootstrap(product)
            original = copy.deepcopy(product.editor)
            saved = data(f'decision connections apply --id {service_id} --name Removal --provider compatible '
                         f'--model fixture --endpoint https://decision.example.test/decide '
                         f'--wire-protocol system_one --source {product.source_id}')
            service = saved['normalized_spec']['desired_state']['change']['service']
            assert service['schema'] == 'hiroute.decision-service/v3'
            apply_service(saved, case + '-save')
            count = operation_count(product)
            assert removal_preview()['blockers'] == []
            assert operation_count(product) == count, 'preview created an Operation'
            selection = copy.deepcopy(original['candidates'][0])
            routing = {'classifier': {'kind': 'decision_service_v3', 'service': service},
                       'branches': [{'id': 'work', 'name': 'Work', 'condition': 'Implementation tasks',
                                     'candidates': [selection], 'primary_candidates': [], 'judgment': None},
                                    {'id': 'docs', 'name': 'Docs', 'condition': 'Documentation tasks',
                                     'candidates': [selection], 'primary_candidates': [], 'judgment': None}],
                       'default_branch_id': 'work', 'judgment': judgment_fixture(),
                       'reselect_on_user_message': False}
            product.editor = dict(original, mode='custom_branches', branch_routing=routing)
            change = plan_change(product, 'update')
            preview = product.preview('routing preview', {'change': change})
            product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, case + '-referenced-plan')
            active = removal_preview()
            assert active['failure'] == 'DECISION_IN_USE'
            assert {'current_head', 'active_publication'} <= {item['protection'] for item in active['blockers']}, active
            refused = data(f'decision connections apply --id {service_id} --delete')
            count = operation_count(product)
            code, error = product.public_cli('decision connections apply', {
                'schema_version': V1, 'spec': refused['normalized_spec'], 'accept_digest': refused['change_digest'],
                'expected_revisions': refused['expected_revisions'], 'idempotency_key': case + '-blocked'}, success=False)
            assert code == 3 and error['error']['code'] == 'REVISION_CONFLICT', error
            assert operation_count(product) == count, 'blocked admission created an Operation'
            if deleted:
                head = read_content(product)[0]
                change = {'schema': 'hiroute.plan-lifecycle-change/v1', 'plan_id': product.plan_id,
                          'expected_head_revision': head['head_revision'], 'status': 'deleted'}
            else:
                product.editor = copy.deepcopy(original)
                change = plan_change(product, 'update')
            preview = product.preview('routing preview', {'change': change})
            product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, case + '-release-current')
            held = removal_preview()
            assert held['failure'] == 'DECISION_IN_USE'
            assert 'lkg_publication' in {item['protection'] for item in held['blockers']}, held
            assert 'current_head' not in {item['protection'] for item in held['blockers']}, held
            checkpoint = {'schema': 'hiroute.publication-checkpoint-change/v1'}
            preview = product.preview('routing preview', {'change': checkpoint})
            product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': checkpoint}, case + '-checkpoint')
            assert removal_preview()['blockers'] == []
            before = rows()
            source_before = source()
            apply_service(data(f'decision connections apply --id {service_id} --delete'), case + '-remove')
            assert rows() == before, 'removal changed history, heads or reference indexes'
            assert not data('decision connections list')['services']
            source_after = source()
            assert source_before['models'] == source_after['models']
            assert source_before['keys'] == source_after['keys']
            assert not any(consumer['service_id'] == service_id for target in source_after['decision_targets']
                           for consumer in target['consumers'])
            product.stop()
            product.start()
            assert rows() == before, 'daemon reopen changed retained history'
            assert not data('decision connections list')['services']
            if deleted:
                product.editor = dict(original, custom_alias=product.model_alias)
                change = plan_change(product, 'create', case + '-alias-reuse')
                code, result = product.public_cli('routing preview', {'change': change}, success=False)
                assert code != 0, result
                assert read_content(product)[0]['status'] == 'deleted'
            else:
                current = data('routing show ' + product.plan_id)
                assert current['desired']['mode'] == original['mode']
            print(json.dumps({'scenario': 'decision-removal-' + case + '-history-restart',
                              'state': 'green', 'legacy_error': 'REVISION_CONFLICT',
                              'historical_rows': len(before['plan_versions']),
                              'shared_models_and_keys_retained': True}), flush=True)
        finally:
            product.close()


if __name__ == '__main__':
    run(sys.argv[1])
