import { test } from 'node:test';
import assert from 'node:assert/strict';
import { trustLevelLabel } from '../src/features/trust/levels.ts';

test('labels preserve exact custom values and recognize the existing default', () => {
  assert.equal(trustLevelLabel(800, 'en'), 'Familiar');
  assert.equal(trustLevelLabel(720, 'en'), 'Custom 0.72');
  assert.equal(trustLevelLabel(720, 'zh'), '自定义 0.72');
  assert.equal(trustLevelLabel(0, 'en'), 'Custom 0');
});
