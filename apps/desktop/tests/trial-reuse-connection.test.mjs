import assert from 'node:assert/strict';
import test from 'node:test';
import { requestModelDefault, savedRequestModel } from '../src/features/trial/reuse-connection.ts';

test('default uses exact saved identity before the selected product mapping', () => {
  const source = { models: [{catalog_configuration_id:'catalog.target',upstream_model_id:'saved/alias'}],connection_identity:{connection_option_id:'product.a'} };
  const options = {options:[{connection_option_id:'product.a',request_model_ids:{'catalog.target':['product/alias']}}]};
  assert.equal(requestModelDefault(source,'catalog.target',options),'saved/alias');
  assert.equal(requestModelDefault({...source,models:[]},'catalog.target',options),'product/alias');
  assert.equal(requestModelDefault({...source,models:[],connection_identity:{connection_option_id:'product.b'}},'catalog.target',options),'');
});

test('names, internal IDs, ambiguous aliases and unrelated products are never guessed', () => {
  const source={models:[{display_name:'Target',upstream_model_id:'unlinked',binding_id:'binding/a'}],connection_identity:{connection_option_id:'product.a'}};
  assert.equal(requestModelDefault(source,'catalog.target',{options:[{connection_option_id:'product.a',known_models:{'catalog.target':'Target'},model_configuration_ids:['catalog.target']}]}),'');
  assert.equal(requestModelDefault(source,'catalog.target',{options:[{connection_option_id:'product.a',request_model_ids:{'catalog.target':['alias/one','alias/two']}}]}),'');
  assert.equal(requestModelDefault({...source,models:[{catalog_configuration_id:'catalog.target',upstream_model_id:'one'},{catalog_configuration_id:'catalog.target',upstream_model_id:'two'}]},'catalog.target'),'');
});

test('edited request ID resolves only one exact saved model', () => {
  const source={models:[{upstream_model_id:'api/model',binding_id:'binding/a'},{upstream_model_id:'other',binding_id:'binding/b'}]};
  assert.equal(savedRequestModel(source,'api/model').binding_id,'binding/a');
  assert.equal(savedRequestModel(source,'api/model '),undefined);
  assert.equal(savedRequestModel({...source,models:[...source.models,source.models[0]]},'api/model'),undefined);
});
