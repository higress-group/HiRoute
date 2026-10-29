import assert from 'node:assert/strict';
import test from 'node:test';
import { modelAnnouncements } from '../src/features/model-feed/announcements.ts';

test('unresolved announcements do not fall back to unrelated directory entries', () => {
  const result = modelAnnouncements({ featured: [], models: [{ model_configuration_id: 'unrelated' }] });
  assert.deepEqual(result, { fromFeed: false, entries: [] });
});

test('only resolved entries in the current feed editorial selection are shown', () => {
  const entry = { id: 'future', model_configuration_id: 'future-model' };
  assert.deepEqual(modelAnnouncements({ featured: [entry], models: [{ model_configuration_id: 'model.deepseek.v4-1-flash' }] }), { fromFeed: true, entries: [] });
  assert.deepEqual(modelAnnouncements({ featured: [entry], models: [{ model_configuration_id: 'future-model' }] }), { fromFeed: true, entries: [entry] });
});

test('an absent or cleared feed selection stays empty even when previously featured models are available', () => {
  const flash = { model_configuration_id: 'model.deepseek.v4-1-flash', availability: { kind: 'executable' } };
  const result = modelAnnouncements({ featured: [], models: [flash, { model_configuration_id: 'unrelated' }] });
  assert.deepEqual(result, { fromFeed: false, entries: [] });
});

test('a later empty snapshot withdraws all previously displayed recommendations', () => {
  const model = { model_configuration_id: 'model.deepseek.v4-1-flash' };
  const entry = { id: 'announced', model_configuration_id: model.model_configuration_id };
  assert.deepEqual(modelAnnouncements({ featured: [entry], models: [model] }).entries, [entry]);
  assert.deepEqual(modelAnnouncements({ featured: [], models: [model] }).entries, []);
});
