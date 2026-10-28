"""Released Agent writers -> current startup -> reopen, without model inference.

Runs only on explicit request. All daemons, native files, credentials and CPA are
isolated fixtures; archives are authenticated against the recorded public snapshot.
No business database rows are manufactured by the candidate serializer.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import sys
import tarfile
import tempfile
import urllib.request

from publication_process import bootstrap
from publication_product import Product
from released_grant_upgrade import upstream_fixture

RELEASE_FIXTURES = {
    'd625e653048b2306c88da07826d52027e9891675': ('8.0.4-hiroute.2', 'fa01e83e8f2a9e1919b618a414e5e6cd3dcbbd23342d66144e9a88e26e447d2c'),
    'ce4c480256c7590aacba080e701d7413a6736426': ('8.0.4-hiroute.4', '0975093534e2d60c7dbfc4606b20cdc5f76d752534859962b21deddce2cd9273'),
    '13856abd873da5b4373d39322493b4cc105c4867': ('8.0.4-hiroute.4', '0975093534e2d60c7dbfc4606b20cdc5f76d752534859962b21deddce2cd9273'),
}


def released_upstream_fixture(repository, revision):
    """Reconstruct and verify the exact released fixture without requiring Git history."""
    version, expected = RELEASE_FIXTURES[revision]
    current = (Path(repository) / 'crates/daemon/tests/support/cpa_upstream.py').read_bytes()
    released, count = re.subn(rb"(?m)^VERSION = '[^']+'$",
                              b"VERSION = '" + version.encode('ascii') + b"'", current)
    assert count == 1 and hashlib.sha256(released).hexdigest() == expected, 'released fixture bytes changed'
    return released


def schema_versions(product):
    versions = []
    for name in ('control', 'runtime', 'secrets'):
        with sqlite3.connect((product.storage / 'live' / (name + '.db')).as_uri() + '?mode=ro', uri=True) as db:
            versions.append(db.execute('PRAGMA user_version').fetchone()[0])
    return versions


def run_source(repository, release_file, current_bin, scratch):
    record = json.loads(release_file.read_text())
    release = record['release']
    artifact = next(item for item in release['artifacts']
                    if item['filename'].endswith('x86_64-unknown-linux-gnu.tar.gz'))
    archive = scratch / artifact['filename']
    url = 'https://github.com/{}/releases/download/{}/{}'.format(
        release['repository'], release['tag'], artifact['filename'])
    request = urllib.request.Request(url, headers={'User-Agent': 'HiRoute-compatibility-test'})
    with urllib.request.urlopen(request, timeout=120) as response, archive.open('wb') as output:
        digest = hashlib.sha256()
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            output.write(chunk)
    assert digest.hexdigest() == artifact['sha256'], 'released package digest mismatch'

    product = Product(repository)
    try:
        released = product.root / 'released'
        released.mkdir(mode=0o700)
        with tarfile.open(archive) as package:
            for member in package.getmembers():
                path = Path(member.name)
                assert not path.is_absolute() and '..' not in path.parts
                assert member.isfile() or member.isdir(), 'unexpected release archive member'
            package.extractall(released)
        product.bin = released / 'bin'
        product.release_version = release['tag'][1:]
        product.enable_cpa()
        upstream_fixture(product, released_upstream_fixture(repository, release['revision']))
        bootstrap(product)
        assert schema_versions(product) == [record['storage']['sql_schema_version']] * 3
        context = product.agent_context_id
        original = product.cli('agents connect status ' + context)[1]['data']
        assert original['state'] == 'configured' and original['current_selection']
        native_before = product.codex_settings.read_bytes()
        product.stop()

        product.bin = current_bin
        product.release_version = json.loads((Path(repository) / 'contracts/cli/local-control-hello.v2.schema.json').read_text())['properties']['client_version']['const']
        product.env['HIROUTE_RECOVERY_PACKAGE'] = str(released)
        upstream_fixture(product, (Path(repository) / 'crates/daemon/tests/support/cpa_upstream.py').read_bytes())
        for _ in range(2):
            product.start()
            assert schema_versions(product) == [33, 33, 33]
            actual = product.cli('agents connect status ' + context)[1]['data']
            assert actual['state'] == 'configured'
            assert actual['current_selection'] == original['current_selection']
            assert product.codex_settings.read_bytes() == native_before
            product.stop()
        attempts = product.cpa_fixture / 'attempts.jsonl'
        assert not attempts.exists() or not attempts.read_bytes(), 'unexpected fixture inference request'
        print(json.dumps({'scenario': 'released-agent-locator-upgrade', 'state': 'green',
                          'source': release['tag'], 'revision': release['revision'],
                          'package_sha256': artifact['sha256'], 'source_schema': record['storage']['sql_schema_version'],
                          'target_schema': 33, 'current_starts': 2, 'model_inference_calls': 0}), flush=True)
    finally:
        product.close()


def run(repository):
    assert sys.platform.startswith('linux')
    current_bin = Path(os.environ['HIROUTE_VALIDATION_PRODUCT_BIN_DIR']).resolve()
    assert (current_bin / 'hirouted').is_file()
    with tempfile.TemporaryDirectory(prefix='hiroute-release-upgrade-') as directory:
        scratch = Path(directory)
        for tag in ('v0.2.0', 'v0.3.0', 'v0.3.1'):
            run_source(repository, Path(repository) / 'contracts/releases' / (tag + '.json'), current_bin, scratch)


if __name__ == '__main__':
    run(sys.argv[1])
