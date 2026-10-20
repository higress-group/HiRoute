import assert from 'node:assert/strict';
import test from 'node:test';
import { readCurrentCatalog } from '../src/features/model-feed/catalog-read.ts';

test('a pure read recaptures after a transient catalog conflict', async () => {
  let calls = 0;
  const result = await readCurrentCatalog(async () => {
    if (++calls < 3) throw { code: 'CHANGE_PREVIEW_STALE' };
    return { generation: 2, models: ['current'] };
  });
  assert.deepEqual(result, { generation: 2, models: ['current'] });
  assert.equal(calls, 3);
});

test('persistent conflicts are bounded and service/authorization failures are not retried', async () => {
  for (const code of ['CHANGE_PREVIEW_STALE', 'DAEMON_UNAVAILABLE', 'CAPABILITY_DENIED']) {
    let calls = 0;
    const error = { code };
    await assert.rejects(readCurrentCatalog(async () => { calls++; throw error; }), value => value === error);
    assert.equal(calls, code === 'CHANGE_PREVIEW_STALE' ? 3 : 1);
  }
});
