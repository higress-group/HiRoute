"""Prepared public CLI requests retain one Operation through real daemon interruption."""
import json
import sys
from publication_product import Product
from publication_process import bootstrap
from plan_content_process import read_content, operation_count


def saved_sources(product):
    sources = product.public_cli('compute list')[1]['data']['sources']
    for source in sources:
        for model in source['models']:
            # Availability is re-evaluated on reads; its timestamp is not saved content.
            model['presentation'].pop('evaluated_at_ms')
    return sources


def scenario(repository, boundary):
    product = Product(repository)
    product.collaboration_only = True
    try:
        bootstrap(product)
        before_head, before_versions = read_content(product)
        before_count = operation_count(product)
        before_sources = saved_sources(product)
        product.stop()
        product.start(boundary)
        path = product.project / 'exact-quickstart-apply.json'
        prepared = product.public_cli(
            f'routing preview --plan {product.plan_id} --purpose RecoveredPurpose --apply-file {path}'
        )[1]['data']
        exact = path.read_bytes()
        submission = prepared['submission']
        command = ' '.join(submission['arguments'])
        code, unknown = product.public_cli(command, success=False)
        assert code != 0 and unknown['status'] != 'succeeded', unknown
        product.stop(crash=True)
        product.start()
        found = product.public_cli('operations find', submission['operation_lookup'])[1]['data']
        assert found['digest_matches'], found
        identity = found['operation']['operation_id']
        for _ in range(2):
            result = product.public_cli(command)[1]
            assert result['operation']['operation_id'] == identity, result
            assert result['operation']['state'] == 'succeeded', result
        after_head, after_versions = read_content(product)
        assert after_head['head_revision'] == before_head['head_revision'] + 1
        assert after_versions[:-1] == before_versions
        assert after_versions[-1]['configuration']['purpose'] == 'RecoveredPurpose'
        assert after_versions[-1]['configuration']['limits'] == before_versions[-1]['configuration']['limits']
        assert operation_count(product) == before_count + 1
        assert saved_sources(product) == before_sources
        assert path.read_bytes() == exact
        print(json.dumps({'scenario': 'quickstart-exact-file-interruption', 'boundary': boundary,
                          'state': 'green', 'new_operations': 1, 'existing_sources_preserved': True}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    for boundary in ['before_install', 'after_terminal']:
        scenario(sys.argv[1], boundary)
