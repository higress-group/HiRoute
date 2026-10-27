import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatEstimateMoney } from '../src/features/estimate-money.ts';
import { formatPresentedCostAmount, presentTrustCost } from '../src/features/trust/cost-presentation.ts';

const comparison = (overrides = {}) => ({ currency: 'USD', coverage: 'unknown', request_count: 1,
  compared_request_count: 0, missing_request_count: 1, excluded_request_count: 0,
  routed_estimate_micros: 2764, reference_estimate_micros: null,
  same_usage_bounds: { reference_min_micros: 92400, reference_max_micros: 115200,
    difference_min_micros: -112436, difference_max_micros: -89636 },
  compared_routed_subtotal_micros: null, compared_reference_subtotal_micros: null,
  retention_boundary_partial: false, arithmetic_overflow: false, ...overrides });
const present = (overrides = {}, difference_micros = null) => presentTrustCost({ kind: 'available', difference_micros, comparison: comparison(overrides) });
const unknown = { kind: 'unknown' };

test('integer money precision keeps tiny amounts, zero, currency and language distinct', () => {
  for (const language of ['zh', 'en']) {
    for (const [micros, expected] of [[0, '$0.00'], [1, '$0.000001'], [49, '$0.000049'], [99, '$0.000099'],
      [100, '$0.0001'], [2764, '$0.0028'], [999999, '$1.0000'], [1000000, '$1.00']]) {
      assert.equal(formatEstimateMoney(micros, ' usd ', language), expected);
    }
    assert.equal(formatEstimateMoney(-49, 'USD', language), '−$0.000049');
    assert.equal(formatEstimateMoney(1, 'USD', language, { signed: true }), '+$0.000001');
    assert.equal(formatEstimateMoney(0, 'USD', language, { signed: true }), '$0.00');
    assert.equal(formatEstimateMoney(1000000, 'abc', language), 'ABC 1.00');
    assert.equal(formatEstimateMoney(1, 'CNY', language), '¥0.000001');
  }
  for (const value of [null, undefined, NaN, Infinity, -Infinity, 1.1, Number.MAX_SAFE_INTEGER + 1]) assert.equal(formatEstimateMoney(value, 'USD', 'en'), null);
  for (const currency of [null, '', 'US', 'USDD', '€UR', 'US1']) assert.equal(formatEstimateMoney(1, currency, 'en'), null);
});

test('signed outward rounding encloses both positive and negative endpoints', () => {
  assert.equal(formatEstimateMoney(112436, 'USD', 'en', { rounding: 'lower', signed: true }), '+$0.1124');
  assert.equal(formatEstimateMoney(112436, 'USD', 'en', { rounding: 'upper', signed: true }), '+$0.1125');
  assert.equal(formatEstimateMoney(-112436, 'USD', 'en', { rounding: 'lower', signed: true }), '−$0.1125');
  assert.equal(formatEstimateMoney(-89636, 'USD', 'en', { rounding: 'upper', signed: true }), '−$0.0896');
  assert.equal(formatEstimateMoney(-112450, 'USD', 'en'), '−$0.1125');
  assert.equal(formatEstimateMoney(Number.MAX_SAFE_INTEGER, 'USD', 'en'), '$9007199254.74');
});

test('a complete same-usage range wins over unknown despite zero exact comparisons', () => {
  const value = present();
  assert.deepEqual(value.routed, { kind: 'exact', micros: 2764 });
  assert.deepEqual(value.reference, { kind: 'range', lowerMicros: 92400, upperMicros: 115200 });
  assert.deepEqual(value.difference, { kind: 'range', lowerMicros: -112436, upperMicros: -89636 });
  assert.equal(value.boundReason, 'cache_write_split');
  assert.deepEqual(value.issues, []);
  assert.equal(formatPresentedCostAmount(value.reference, value.currency, 'en'), '$0.0924 to $0.1152');
  assert.equal(formatPresentedCostAmount(value.difference, value.currency, 'zh', true), '−$0.1125 至 −$0.0896');
});

test('exact values take precedence; a known routed amount survives an unknown reference', () => {
  const exact = present({ reference_estimate_micros: 3000, same_usage_bounds: { reference_min_micros: -1 } }, -236);
  assert.deepEqual(exact.reference, { kind: 'exact', micros: 3000 });
  assert.deepEqual(exact.difference, { kind: 'exact', micros: -236 });
  assert.equal(exact.boundReason, null);
  assert.deepEqual(exact.issues, []);
  const routed = present({ reference_estimate_micros: null, same_usage_bounds: null });
  assert.deepEqual(routed.routed, { kind: 'exact', micros: 2764 });
  assert.deepEqual(routed.reference, unknown);
  assert.deepEqual(routed.difference, unknown);
  assert.deepEqual(routed.issues, []);
  const noDifference = present({ reference_estimate_micros: 3000, same_usage_bounds: null });
  assert.deepEqual(noDifference.difference, unknown, 'A missing exact difference is not synthesized');
});

test('bad bounds cannot replace an unknown total, and do not erase a valid routed total', () => {
  const valid = comparison().same_usage_bounds;
  for (const bounds of [
    { ...valid, reference_min_micros: undefined }, { ...valid, reference_min_micros: 120000 },
    { ...valid, difference_min_micros: -1 }, { ...valid, difference_max_micros: Number.MAX_SAFE_INTEGER + 1 },
    { ...valid, reference_min_micros: -1 }, { ...valid, reference_max_micros: 115200.5 },
  ]) {
    const result = present({ same_usage_bounds: bounds });
    assert.deepEqual(result.routed, { kind: 'exact', micros: 2764 });
    assert.deepEqual(result.reference, unknown);
    assert.deepEqual(result.difference, unknown);
    assert.deepEqual(result.issues, ['invalid_bounds']);
  }
});

test('count inconsistency, retention and overflow never promote paired subtotals to totals', () => {
  for (const overrides of [
    { request_count: -1 }, { compared_request_count: 2 }, { missing_request_count: 0 },
    { excluded_request_count: 2 }, { request_count: 1.5 },
    { retention_boundary_partial: true }, { arithmetic_overflow: true },
  ]) {
    const result = present({ compared_routed_subtotal_micros: 10, compared_reference_subtotal_micros: 20, ...overrides });
    assert.deepEqual([result.routed, result.reference, result.difference], [unknown, unknown, unknown]);
    assert.equal(result.boundReason, null);
    assert.equal(result.issues.length, 1);
  }
  assert.deepEqual(present({ retention_boundary_partial: true, arithmetic_overflow: true, request_count: -1 }).issues,
    ['invalid_counts', 'retention', 'overflow']);
  assert.deepEqual(present({ routed_estimate_micros: -1, same_usage_bounds: null }).issues, ['invalid_amount']);
});

test('unavailable reasons and invalid currency stay explicit', () => {
  assert.deepEqual(presentTrustCost({ kind: 'unavailable', reason: 'primary_not_unique' }), { kind: 'unavailable', reason: 'primary_not_unique' });
  assert.deepEqual(present({ currency: '' }), { kind: 'unavailable', reason: 'currency_not_unique' });
});

test('range arithmetic keeps signs when the difference is positive or crosses zero', () => {
  for (const [routed, min, max, lower, upper, text] of [
    [500000, 100000, 300000, 200000, 400000, '+$0.2000 to +$0.4000'],
    [200000, 100000, 300000, -100000, 100000, '−$0.1000 to +$0.1000'],
    [200000, 200000, 200000, 0, 0, '$0.00 to $0.00'],
  ]) {
    const result = present({ routed_estimate_micros: routed, same_usage_bounds: { reference_min_micros: min,
      reference_max_micros: max, difference_min_micros: lower, difference_max_micros: upper } });
    assert.equal(formatPresentedCostAmount(result.difference, result.currency, 'en', true), text);
  }
});
