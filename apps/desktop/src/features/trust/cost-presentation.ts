import { formatEstimateMoney } from '../estimate-money.ts';
import type { TrustCostValue } from './TrustCost';

export type PresentedCostAmount =
  | { kind: 'exact'; micros: number }
  | { kind: 'range'; lowerMicros: number; upperMicros: number }
  | { kind: 'unknown' };
export type CostPresentationIssue = 'invalid_counts' | 'invalid_bounds' | 'invalid_amount' | 'retention' | 'overflow';
export type CostPresentation =
  | { kind: 'unavailable'; reason: string }
  | { kind: 'available'; currency: string; routed: PresentedCostAmount; reference: PresentedCostAmount;
      difference: PresentedCostAmount; boundReason: 'cache_write_split' | null; issues: readonly CostPresentationIssue[] };

const unknown: PresentedCostAmount = { kind: 'unknown' };
const safe = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const nonnegative = (value: unknown): value is number => safe(value) && value >= 0;
const issueOrder: readonly CostPresentationIssue[] = ['invalid_counts', 'invalid_bounds', 'invalid_amount', 'retention', 'overflow'];

/** Select only amounts that describe the full requested comparison population. */
export function presentTrustCost(value: TrustCostValue): CostPresentation {
  if (value.kind === 'unavailable') return value;
  const c = value.comparison;
  const currency = typeof c.currency === 'string' ? c.currency.trim().toUpperCase() : '';
  if (!/^[A-Z]{3}$/.test(currency)) return { kind: 'unavailable', reason: 'currency_not_unique' };
  const issues = new Set<CostPresentationIssue>();
  const validCounts = [c.request_count, c.compared_request_count, c.missing_request_count, c.excluded_request_count].every(nonnegative)
    && c.compared_request_count <= c.request_count && c.excluded_request_count <= c.missing_request_count
    && c.missing_request_count === c.request_count - c.compared_request_count;
  if (!validCounts) issues.add('invalid_counts');
  if (c.retention_boundary_partial) issues.add('retention');
  if (c.arithmetic_overflow) issues.add('overflow');
  const result = (routed: PresentedCostAmount, reference: PresentedCostAmount, difference: PresentedCostAmount): CostPresentation => ({
    kind: 'available', currency, routed, reference, difference,
    boundReason: reference.kind === 'range' || difference.kind === 'range' ? 'cache_write_split' : null,
    issues: issueOrder.filter(issue => issues.has(issue)),
  });
  if (!validCounts || c.retention_boundary_partial || c.arithmetic_overflow) return result(unknown, unknown, unknown);

  if (c.routed_estimate_micros != null && !nonnegative(c.routed_estimate_micros)) issues.add('invalid_amount');
  if (c.reference_estimate_micros != null && !nonnegative(c.reference_estimate_micros)) issues.add('invalid_amount');
  const routed: PresentedCostAmount = nonnegative(c.routed_estimate_micros) ? { kind: 'exact', micros: c.routed_estimate_micros } : unknown;
  const exactReference: PresentedCostAmount = nonnegative(c.reference_estimate_micros) ? { kind: 'exact', micros: c.reference_estimate_micros } : unknown;
  if (routed.kind === 'exact' && exactReference.kind === 'exact' && value.difference_micros != null && !safe(value.difference_micros)) issues.add('invalid_amount');
  const exactDifference: PresentedCostAmount = routed.kind === 'exact' && exactReference.kind === 'exact' && safe(value.difference_micros)
    ? { kind: 'exact', micros: value.difference_micros } : unknown;

  let reference = exactReference, difference = exactDifference;
  const bounds = c.same_usage_bounds;
  if ((reference.kind !== 'exact' || difference.kind !== 'exact') && bounds != null) {
    const { reference_min_micros: min, reference_max_micros: max, difference_min_micros: diffMin, difference_max_micros: diffMax } = bounds;
    const valid = routed.kind === 'exact' && nonnegative(min) && nonnegative(max) && safe(diffMin) && safe(diffMax)
      && min <= max && diffMin <= diffMax
      && BigInt(diffMin) === BigInt(routed.micros) - BigInt(max)
      && BigInt(diffMax) === BigInt(routed.micros) - BigInt(min);
    if (valid) {
      if (reference.kind !== 'exact') reference = { kind: 'range', lowerMicros: min, upperMicros: max };
      if (difference.kind !== 'exact') difference = { kind: 'range', lowerMicros: diffMin, upperMicros: diffMax };
    } else issues.add('invalid_bounds');
  }
  return result(routed, reference, difference);
}

export function formatPresentedCostAmount(amount: PresentedCostAmount, currency: string, language: 'zh' | 'en', signed = false): string {
  if (amount.kind === 'unknown') return language === 'zh' ? '未知' : 'Unknown';
  if (amount.kind === 'exact') return formatEstimateMoney(amount.micros, currency, language, { signed }) ?? (language === 'zh' ? '未知' : 'Unknown');
  const lower = formatEstimateMoney(amount.lowerMicros, currency, language, { rounding: 'lower', signed });
  const upper = formatEstimateMoney(amount.upperMicros, currency, language, { rounding: 'upper', signed });
  return lower && upper ? `${lower}${language === 'zh' ? ' 至 ' : ' to '}${upper}` : (language === 'zh' ? '未知' : 'Unknown');
}

export function trustCostUnavailableReason(reason: string, language: 'zh' | 'en'): string {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  return reason === 'primary_not_unique' ? t('需要唯一的已发布主力模型。', 'A unique published Primary model is required.')
    : reason === 'currency_not_unique' ? t('需要唯一的可用 API 参考币种。', 'One available API-reference currency is required.')
    : reason === 'prices_unavailable' || reason === 'reference_unavailable' ? t('当前来源的 API 参考价格不可用。', 'Current source-specific API reference prices are unavailable.')
    : t('记录正在准备或证据不足，请稍后重试。', 'Records are catching up or evidence is unavailable. Retry later.');
}
