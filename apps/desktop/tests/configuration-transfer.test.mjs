import assert from 'node:assert/strict';
import test from 'node:test';

import { parseConfigurationBundle, validConfigurationBundle } from '../src/features/configuration-transfer.ts';

function bundle(overrides = {}) {
  return {
    schema: 'hiroute.configuration-bundle/v1',
    worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 4 },
    observation_capacity: { max_active_bytes: 1_048_576 },
    model_feed: { retrieval_enabled: true, acknowledge_disclosure: true },
    ...overrides,
  };
}

test('a complete exported bundle is accepted', () => {
  assert.equal(validConfigurationBundle(bundle()), true);
});

test('a disabled record capacity is accepted as null', () => {
  assert.equal(validConfigurationBundle(bundle({ observation_capacity: { max_active_bytes: null } })), true);
});

test('a non-current schema is rejected', () => {
  assert.equal(validConfigurationBundle(bundle({ schema: 'not-the-configuration-bundle-schema' })), false);
});

test('worker concurrency outside 1-1000 is rejected', () => {
  assert.equal(validConfigurationBundle(bundle({ worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 0 } })), false);
  assert.equal(validConfigurationBundle(bundle({ worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 1001 } })), false);
  assert.equal(validConfigurationBundle(bundle({ worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 1 } })), true);
  assert.equal(validConfigurationBundle(bundle({ worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 1000 } })), true);
});

test('a worker settings schema mismatch is rejected', () => {
  assert.equal(validConfigurationBundle(bundle({ worker_settings: { schema: 'not-the-worker-settings-schema', max_concurrent: 4 } })), false);
});

test('a non-positive or non-integer capacity is rejected', () => {
  assert.equal(validConfigurationBundle(bundle({ observation_capacity: { max_active_bytes: 0 } })), false);
  assert.equal(validConfigurationBundle(bundle({ observation_capacity: { max_active_bytes: -1 } })), false);
  assert.equal(validConfigurationBundle(bundle({ observation_capacity: { max_active_bytes: 1.5 } })), false);
});

test('missing model feed flags are rejected', () => {
  assert.equal(validConfigurationBundle(bundle({ model_feed: {} })), false);
  assert.equal(validConfigurationBundle(bundle({ model_feed: { retrieval_enabled: true } })), false);
  assert.equal(validConfigurationBundle(bundle({ model_feed: { retrieval_enabled: 'true', acknowledge_disclosure: true } })), false);
});

test('missing sections and non-objects are rejected', () => {
  assert.equal(validConfigurationBundle(undefined), false);
  assert.equal(validConfigurationBundle(null), false);
  assert.equal(validConfigurationBundle(''), false);
  assert.equal(validConfigurationBundle([]), false);
  const value = bundle(); delete value.observation_capacity;
  assert.equal(validConfigurationBundle(value), false);
});

test('a JSON round trip keeps the bundle admissible', () => {
  const restored = JSON.parse(JSON.stringify(bundle()));
  assert.equal(validConfigurationBundle(restored), true);
});

test('parsing admits a complete exported file', () => {
  assert.deepEqual(parseConfigurationBundle(JSON.stringify(bundle())), bundle());
});

test('parsing rejects malformed JSON without throwing', () => {
  assert.equal(parseConfigurationBundle('{'), undefined);
  assert.equal(parseConfigurationBundle(''), undefined);
  assert.equal(parseConfigurationBundle('not json'), undefined);
});

test('parsing rejects an inadmissible bundle', () => {
  assert.equal(parseConfigurationBundle(JSON.stringify(bundle({ schema: 'not-the-configuration-bundle-schema' }))), undefined);
  assert.equal(parseConfigurationBundle(JSON.stringify(bundle({ worker_settings: { schema: 'hiroute.worker-settings/v1', max_concurrent: 0 } }))), undefined);
});

test('parsing keeps a disabled capacity admissible', () => {
  const value = bundle({ observation_capacity: { max_active_bytes: null } });
  assert.deepEqual(parseConfigurationBundle(JSON.stringify(value)), value);
});
