import test from 'node:test';
import assert from 'node:assert/strict';
import { buildModelLabelIndex, modelLabel } from '../src/features/model-labels.ts';

test('exact identities reject conflicts and do not borrow names', () => {
  const labels = buildModelLabelIndex([
    { model_configuration_id:'model/a', binding_id:'a', display_name:'Astra' },
    { model_configuration_id:'model/b', binding_id:'b', display_name:'Flash' },
    { model_configuration_id:'model/b', binding_id:'c', display_name:'Other' },
  ]);
  assert.equal(modelLabel('model/a', 'native/a', labels, 'en').text, 'Astra');
  assert.equal(modelLabel('model/a-extra', 'native/other', labels, 'en').text, 'native/other');
  assert.equal(modelLabel('model/b', 'native/b', labels, 'en').text, 'native/b');
  assert.deepEqual(modelLabel('deleted', null, labels, 'en'), { text:'Model unconfirmed', detail:'Configuration ID: deleted', basis:'unavailable' });
});

test('duplicate names use only proven source membership and unique ID prefixes', () => {
  const candidates = [
    { model_configuration_id:'abcdefgh-one', binding_id:'a', display_name:'Astra' },
    { model_configuration_id:'abcdefgh-two', binding_id:'b', display_name:'Astra' },
  ];
  let labels = buildModelLabelIndex(candidates);
  assert.equal(modelLabel('abcdefgh-one', null, labels, 'en').text, 'Astra · Source unconfirmed · abcdefgh-o');
  labels = buildModelLabelIndex(candidates, [
    { source_id:'one', display_name:'First', binding_ids:['a'] },
    { source_id:'two', display_name:'Second', binding_ids:['b'] },
  ]);
  assert.equal(modelLabel('abcdefgh-one', null, labels, 'en').text, 'Astra · First');
  labels = buildModelLabelIndex([...candidates, { ...candidates[0], binding_id:'c' }], [
    { source_id:'one', display_name:'First', binding_ids:['a'] },
    { source_id:'two', display_name:'Second', binding_ids:['b','c'] },
  ]);
  assert.equal(modelLabel('abcdefgh-one', null, labels, 'en').text, 'Astra · Multiple connections');
  labels = buildModelLabelIndex([...candidates, { ...candidates[0], binding_id:'missing' }], [{ source_id:'one', display_name:'First', binding_ids:['a'] }]);
  assert.match(modelLabel('abcdefgh-one', null, labels, 'en').text, /Source unconfirmed/);
});

test('many same-name configurations retain distinct visible identities', () => {
  const candidates = Array.from({ length: 400 }, (_, index) => ({
    model_configuration_id: `shared-prefix-${String(index).padStart(4, '0')}`,
    binding_id: `binding-${index}`,
    display_name: 'Shared name',
  }));
  const labels = buildModelLabelIndex(candidates);
  const visible = candidates.map(candidate => modelLabel(candidate.model_configuration_id, null, labels, 'en').text);
  assert.equal(new Set(visible).size, candidates.length);
  assert.ok(visible.every(label => label.includes('Source unconfirmed')));
});
