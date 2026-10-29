import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { connectionDirectory, filterConnectionDirectory } from '../src/features/model-connections/connection-directory.ts';
import { ConnectionEditingSession } from '../src/features/model-connections/connection-session.ts';
import { newModelConnectionDraft } from '../src/features/model-connections/new-draft.ts';
import { blankModel } from '../src/features/model-connections/state.ts';

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

test('only editable values and session-owned input differ from a product baseline', () => {
  const session = new ConnectionEditingSession();
  const draft = { ...newModelConnectionDraft('known'), display_name: 'API' };
  session.begin('api', draft);
  const generated = { ...draft, lineage_ref: 'new-lineage', candidate_ref: 'checked', edit_revision: 7, check_id: 'check' };
  assert.equal(session.changed('api', generated), false);
  assert.equal(session.changed('api', { ...generated, display_name: 'Edited' }), true);
  assert.equal(session.changed('api', { ...generated, display_name: 'API' }), false);
  const model = blankModel('model-a');
  session.applyModelDefaults('api', () => [model]);
  const withModel = { ...generated, models: [model] };
  assert.equal(session.changed('api', withModel), false);
  const changed = { ...model, capabilities: { ...model.capabilities, vision: { value: true, basis: 'user_declared' } } };
  assert.equal(session.changed('api', { ...withModel, models: [changed] }), true);
  const restored = { ...changed, capabilities: { ...changed.capabilities, vision: { ...model.capabilities.vision, basis: 'user_declared' } } };
  assert.equal(session.changed('api', { ...withModel, models: [restored] }), false);
  session.stash('api', { draft: withModel, custom: false, metadataPrefillKey: '', secret: '', protectedInput: null, selection: null });
  assert.equal(session.hasUnsavedDrafts, false);
  session.stash('api', { draft: withModel, custom: false, metadataPrefillKey: '', secret: 'fixture', protectedInput: null, selection: null });
  assert.equal(session.hasUnsavedDrafts, true);
});

test('only an exact pristine automatic model row is ignored', () => {
  const session = new ConnectionEditingSession();
  const initial = newModelConnectionDraft('custom');
  const automatic = blankModel();
  session.begin('custom', initial);
  session.recordAutoPlaceholder('custom', automatic);
  session.applyModelDefaults('custom', () => [automatic]);
  assert.equal(session.changed('custom', initial), false);
  assert.equal(session.changed('custom', { ...initial, models: [automatic] }), false);
  assert.equal(session.changed('custom', { ...initial, models: [{ ...automatic, display_name: 'Partial' }] }), true);
  assert.equal(session.changed('custom', { ...initial, models: [{ ...automatic, capabilities: { ...automatic.capabilities, vision: { value: true, basis: 'user_declared' } } }] }), true);
  assert.equal(session.changed('custom', { ...initial, models: [blankModel()] }), true, 'another user-created blank row is not the automatic placeholder');
  assert.equal(session.changed('custom', { ...initial, models: [blankModel('existing-model')] }), true);
});

test('current and retained products compare against their prefill baseline', () => {
  const session = new ConnectionEditingSession();
  const initial = { ...newModelConnectionDraft('custom'), display_name: 'Custom API' };
  const template = { ...initial, base_url: 'https://example.test/v1', models: [blankModel('catalog-model')] };
  const context = 'template:example';
  session.begin('custom', initial);
  session.beginPrefill('custom', context, template);
  assert.equal(session.changed('custom', template, context), false, 'the current template uses its own defaults');
  session.stash('custom', { draft: template, custom: true, metadataPrefillKey: context, secret: '', protectedInput: null, credentialReentry: false, selection: null });
  assert.equal(session.hasUnsavedDrafts, false, 'the retained product uses the same identity');
});

test('prefill baselines survive revisits and protect edits in every retained context', () => {
  const session = new ConnectionEditingSession();
  const initial = { ...newModelConnectionDraft('custom'), display_name: 'Custom API' };
  const model = blankModel('catalog-model');
  const a = { ...initial, base_url: 'https://a.example.test/v1', models: [model] };
  const b = { ...initial, base_url: 'https://b.example.test/v1' };
  const value = draft => ({ draft, custom: true, metadataPrefillKey: 'a', secret: '', protectedInput: null, credentialReentry: false, selection: null });
  session.begin('custom', initial); session.beginPrefill('custom', 'a', a); session.beginPrefill('custom', 'b', b);
  session.beginPrefill('custom', 'a', { ...a, display_name: 'Edited' });
  assert.equal(session.changed('custom', a, 'a'), false, 'a revisit does not overwrite the first baseline');
  assert.equal(session.changed('custom', { ...a, models: [] }, 'a'), true, 'deleting a prefilled model is a real edit');
  assert.equal(session.changed('custom', { ...a, display_name: 'Edited' }, 'a'), true);
  session.stashPrefill('custom', 'a', value({ ...a, display_name: 'Edited' }));
  assert.equal(session.hasUnsavedDrafts, true, 'an edited parked prefill remains dirty');
  assert.equal(session.takePrefill('custom', 'a').draft.display_name, 'Edited');
  session.stashPrefill('custom', 'a', value(a));
  session.stash('custom', { ...value(b), metadataPrefillKey: 'b' });
  assert.equal(session.hasUnsavedDrafts, false, 'two untouched contexts are clean');
  session.stashPrefill('custom', 'a', { ...value(a), selection: { refs: new Set(['model-ref']) } });
  assert.equal(session.hasUnsavedDrafts, true, 'a parked model selection is protected');
  session.stashPrefill('custom', 'a', { ...value(a), secret: 'fixture-key' });
  assert.equal(session.hasUnsavedDrafts, true, 'a parked Key is protected');
});

test('a product converted to a prefill derives its baseline before user edits', () => {
  const session = new ConnectionEditingSession();
  const product = { ...newModelConnectionDraft('known'), display_name: 'Original' };
  const derive = draft => ({ ...draft, entry_kind: 'custom_api', base_url: 'https://example.test/v1' });
  session.begin('product', product);
  session.beginPrefillFromContext('product', undefined, 'template:product', derive);
  assert.equal(session.changed('product', derive(product), 'template:product'), false);
  assert.equal(session.changed('product', derive({ ...product, display_name: 'Edited' }), 'template:product'), true);
  session.beginPrefillFromContext('product', undefined, 'template:product', draft => ({ ...derive(draft), display_name: 'Edited' }));
  assert.equal(session.changed('product', derive({ ...product, display_name: 'Edited' }), 'template:product'), true, 're-entering cannot reset the baseline');
});

test('a new prefill derives from the source context baseline, not its carried edits', () => {
  const session = new ConnectionEditingSession();
  const original = { ...newModelConnectionDraft('custom'), display_name: 'Original' };
  const derive = draft => ({ ...draft, base_url: 'https://example.test/v1' });
  session.begin('custom', original); session.beginPrefill('custom', '', original);
  session.beginPrefillFromContext('custom', '', 'template:a', derive);
  const carried = derive({ ...original, display_name: 'Edited' });
  assert.equal(session.changed('custom', carried, 'template:a'), true);
  assert.equal(session.changed('custom', derive(original), 'template:a'), false);
  session.beginPrefillFromContext('custom', '', 'template:a', () => carried);
  assert.equal(session.changed('custom', carried, 'template:a'), true, 'a later visit cannot adopt a carried edit as a default');
});

test('check-generated models do not replace a prefill baseline', () => {
  const session = new ConnectionEditingSession();
  const original = { ...newModelConnectionDraft('custom'), models: [blankModel('prefilled')] };
  const generated = blankModel('observed');
  session.begin('custom', original); session.beginPrefill('custom', 'template:a', original);
  session.recordAutomaticModel('custom', 'template:a', generated);
  assert.equal(session.changed('custom', { ...original, models: [...original.models, generated] }, 'template:a'), false);
  assert.equal(session.changed('custom', { ...original, models: [generated] }, 'template:a'), true, 'deleting the original model stays dirty');
  assert.equal(session.changed('custom', original, 'template:a'), true, 'deleting the generated nonblank model stays dirty');
  const modified = { ...generated, display_name: 'Edited' };
  session.recordAutomaticModel('custom', 'template:a', modified);
  assert.equal(session.changed('custom', { ...original, models: [...original.models, modified] }, 'template:a'), true, 'another check cannot overwrite the first generated default');
});
