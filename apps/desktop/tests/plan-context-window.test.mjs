import { test } from 'node:test';
import assert from 'node:assert/strict';
import { contextWindowError } from '../src/plan-context-window.ts';

test('default and custom windows allow the full proven range', () => {
  const bounds = { maximum_tokens: 1050000, default_tokens: 272000 };
  for (const value of [undefined, 1, 128000, 272000, 500000, 1050000]) {
    assert.equal(contextWindowError(value, bounds, 'zh'), null);
  }
  for (const value of [0, -1, 1.5, NaN, Infinity, 1050001]) {
    assert.ok(contextWindowError(value, bounds, 'en'));
  }
});

test('candidate changes invalidate custom values without changing them', () => {
  assert.match(contextWindowError(500000, { maximum_tokens: 64000, default_tokens: 64000 }, 'zh'), /64,000/);
  assert.ok(contextWindowError(500000, null, 'en'));
  assert.equal(contextWindowError(undefined, null, 'en'), null);
});
