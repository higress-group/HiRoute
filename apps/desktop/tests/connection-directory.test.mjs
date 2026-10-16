import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { connectionDirectory, filterConnectionDirectory } from '../src/features/model-connections/connection-directory.ts';
import { ConnectionEditingSession } from '../src/features/model-connections/connection-session.ts';

const read = path => JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8'));
const registry = read('../../../assets/release-facts/current/bundle/connector-registry.json');
const templates = read('../../../assets/model-data/current/runtime-projection.json').connection_templates;
const lookup = id => templates.find(item => item.connection_option_id === id);
const directory = connectionDirectory(registry.connection_options, 'zh', lookup);

test('the directory contains every current API option and excludes subscription login', () => {
  const native = registry.connection_options.filter(item => item.origin !== 'agent_subscription');
  assert.deepEqual(directory.flatMap(provider => provider.products.map(product => product.option.connection_option_id)).sort(),
    native.map(option => option.connection_option_id).sort());
  assert.equal(directory.length, 13);
  assert.equal(native.length, 19);
  assert.ok(native.some(option => option.connection_option_id === 'zhipu.coding-chat.cn.v1' && option.billing_class === 'subscription'));
  assert.equal(directory.find(provider => provider.id === 'bailian').products.length, 3);
  const future = { ...native[0], connection_option_id: 'future.api', connector_id: 'future.connector', display_name: 'Future service' };
  assert.equal(connectionDirectory([...native, future], 'zh', lookup).at(-1).products[0].option.connection_option_id, 'future.api');
});

test('search finds product names in either language and billing filters retain only matching products', () => {
  assert.equal(filterConnectionDirectory(directory, '百炼', 'all')[0].products.length, 3);
  assert.ok(filterConnectionDirectory(directory, 'Coding Plan', 'all').length > 1);
  const free = filterConnectionDirectory(directory, '', 'free');
  assert.equal(free.flatMap(provider => provider.products).length, 5);
  assert.ok(free.every(provider => provider.products.every(product => product.option.billing_class === 'free')));
  assert.equal(filterConnectionDirectory(directory, 'not-a-provider', 'all').length, 0);
});

test('editing credentials stay scoped to a product and unused protected inputs are released', () => {
  const session = new ConnectionEditingSession();
  const input = id => ({ candidate_ref: id, candidate_revision: 1 });
  const first = { draft: { display_name: 'Team' }, custom: false, metadataPrefillKey: '', secret: 'fixture-first', protectedInput: input('first') };
  const second = { draft: { display_name: 'Personal' }, custom: false, metadataPrefillKey: '', secret: 'fixture-second', protectedInput: input('second') };
  session.stash('first', first);
  session.stash('second', second);
  assert.equal(session.take('other'), undefined);
  assert.equal(session.take('first'), first);
  assert.equal(session.take('first'), undefined);
  const released = [];
  session.clear(value => released.push(value.candidate_ref));
  assert.deepEqual(released, ['second']);
  assert.equal(second.secret, '');
  assert.equal(second.protectedInput, null);
  assert.equal(session.hasDrafts, false);
});
