import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DEFAULT_REQUEST_TIMEOUT_MS, requestTimeoutError } from '../src/plan-request-timeout.ts';

test('new plans default to the maximum one-hour request timeout', () => {
  assert.equal(DEFAULT_REQUEST_TIMEOUT_MS, 3_600_000);
  assert.equal(requestTimeoutError(DEFAULT_REQUEST_TIMEOUT_MS, 30_000, 'zh'), null);
});

test('streaming request timeout accepts editable bounds', () => {
  for (const value of [30_000, 60_000, 600_000, 3_600_000]) {
    assert.equal(requestTimeoutError(value, 30_000, 'zh'), null);
  }
  for (const value of [0, 29_999, 3_600_001, 60_000.5, NaN, Infinity]) {
    assert.ok(requestTimeoutError(value, 30_000, 'en'));
  }
});

test('request timeout may not be shorter than the attempt timeout', () => {
  assert.ok(requestTimeoutError(60_000, 90_000, 'zh'));
  assert.equal(requestTimeoutError(90_000, 90_000, 'en'), null);
});
