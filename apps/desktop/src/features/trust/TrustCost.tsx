import { Disclosure } from '../../ui';
import { formatEstimateMoney } from '../estimate-money';
import { formatPresentedCostAmount, presentTrustCost, trustCostUnavailableReason, type PresentedCostAmount } from './cost-presentation';

export type TrustCostValue = { kind: 'unavailable'; reason: string } | {
  kind: 'available'; difference_micros: number | null;
  comparison: { currency: string; coverage: 'complete' | 'partial' | 'unknown'; request_count: number; compared_request_count: number; missing_request_count: number; excluded_request_count: number; routed_estimate_micros: number | null; reference_estimate_micros: number | null; same_usage_bounds?: { reference_min_micros: number; reference_max_micros: number; difference_min_micros: number; difference_max_micros: number } | null; compared_routed_subtotal_micros: number | null; compared_reference_subtotal_micros: number | null; retention_boundary_partial: boolean; arithmetic_overflow: boolean };
};

export function TrustCost({ value, language }: { value: TrustCostValue; language: 'zh' | 'en' }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const title = <h3>{t('费用对比（估算）', 'Cost comparison (estimate)')}</h3>;
  const presented = presentTrustCost(value);
  if (presented.kind === 'unavailable') return <section className="trust-card trust-cost" aria-label={t('常规组成本对比', 'Regular cost comparison')}>{title}<p className="field-help">{t('暂不可用：', 'Unavailable: ')}{trustCostUnavailableReason(presented.reason, language)}</p></section>;
  if (value.kind !== 'available') return null;
  const c = value.comparison;
  const amount = (entry: PresentedCostAmount, signed = false) => formatPresentedCostAmount(entry, presented.currency, language, signed);
  const difference = presented.difference;
  const differenceTone = difference.kind === 'exact' ? difference.micros < 0 ? 'less' : difference.micros > 0 ? 'more' : ''
    : difference.kind === 'range' ? difference.upperMicros < 0 ? 'less' : difference.lowerMicros > 0 ? 'more' : '' : '';
  const row = (label: string, entry: PresentedCostAmount, key: string, signed = false, tone = '') => <div className="trust-cost-row" data-cost-metric={key}>
    <span>{label}</span><strong className={tone}>{entry.kind === 'unknown' && key === 'difference' ? t('差额未知', 'Difference unknown') : amount(entry, signed)}{entry.kind === 'range' && <> <small className="trust-cost-range">{t('范围', 'Range')}</small></>}</strong>
  </div>;
  const validCounts = !presented.issues.includes('invalid_counts');
  const formatSubtotal = (micros: number | null) => Number.isSafeInteger(micros) && (micros as number) >= 0
    ? formatEstimateMoney(micros, presented.currency, language) ?? t('未知', 'Unknown') : t('未知', 'Unknown');
  const hasSubtotal = validCounts && c.compared_request_count > 0
    && [c.compared_routed_subtotal_micros, c.compared_reference_subtotal_micros].some(micros => Number.isSafeInteger(micros) && (micros as number) >= 0);
  return <section className="trust-card trust-cost" aria-label={t('常规组成本对比', 'Regular cost comparison')}>
    <div className="trust-card-head">{title}<span className="trust-cost-currency">{presented.currency}</span></div>
    {row(t('常规阶段估算费用', 'Estimated Regular cost'), presented.routed, 'routed')}
    {row(t('同样用量的主力参考', 'Primary reference for the same usage'), presented.reference, 'reference')}
    {row(t('估算差额', 'Estimated difference'), difference, 'difference', true, differenceTone)}
    <p className="field-help">{t('按同样用量换算的估算，不是账单，也不是实际节省。', 'Estimate for the same usage; not a bill or an actual cash saving.')}</p>
    {presented.issues.includes('retention') && <p className="field-help">{t('部分记录已超出保留期限。', 'Some records are outside retention.')}</p>}
    {presented.issues.includes('overflow') && <p className="field-help">{t('金额超出计算范围。', 'An amount exceeds the arithmetic range.')}</p>}
    {presented.issues.includes('invalid_counts') && <p className="field-help">{t('费用证据暂不一致。', 'Cost evidence is inconsistent.')}</p>}
    {(presented.issues.includes('invalid_bounds') || presented.issues.includes('invalid_amount')) && <p className="field-help">{t('部分费用证据无法核实。', 'Some cost evidence cannot be verified.')}</p>}
    {validCounts && c.missing_request_count > 0 && !presented.boundReason && (presented.reference.kind === 'unknown' || difference.kind === 'unknown')
      && !presented.issues.length && <p className="field-help">{t('所选请求中仍有无法完整估算的部分。', 'Some selected requests cannot be estimated completely.')}</p>}
    <Disclosure className="trust-cost-details" label={t('计算方式', 'Calculation details')} language={language}>
      <p>{t('范围为所选版本、分类和常规组；使用已记录 token，包含重试，不含决策模型调用。', 'The selected version, category and Regular group use recorded tokens, including retries and excluding decision-model calls.')}</p>
      {validCounts ? <p>{t(`可精确比较 ${c.compared_request_count}/${c.request_count} 个请求 · 缺口 ${c.missing_request_count}。`, `Exactly comparable: ${c.compared_request_count}/${c.request_count} requests · ${c.missing_request_count} missing.`)}</p>
        : <p>{t('费用证据暂不一致。', 'Cost evidence is inconsistent.')}</p>}
      {presented.boundReason ? <p>{t('其余按未上报缓存写入量的上下限估算。', 'The remainder is bounded using the possible unreported cache-write amount.')}</p>
        : validCounts && c.missing_request_count > 0 && <p>{t(`还有 ${c.missing_request_count} 个请求不能完整比较。`, `${c.missing_request_count} ${c.missing_request_count === 1 ? 'request cannot' : 'requests cannot'} be completely compared.`)}</p>}
      {presented.boundReason && c.same_usage_bounds && <p>{t('未舍入范围（微单位）：', 'Unrounded bounds (micros): ')}{c.same_usage_bounds.reference_min_micros} / {c.same_usage_bounds.reference_max_micros} · {c.same_usage_bounds.difference_min_micros} / {c.same_usage_bounds.difference_max_micros}</p>}
      {hasSubtotal && <p>{t('仅已知配对请求的小计（常规 / 主力）：', 'Known paired-request subtotals only (Regular / Primary): ')}{formatSubtotal(c.compared_routed_subtotal_micros)} / {formatSubtotal(c.compared_reference_subtotal_micros)}</p>}
      {validCounts && c.excluded_request_count > 0 && <p>{t(`排除 ${c.excluded_request_count} 个请求，已包含在缺口数量中。`, `${c.excluded_request_count} excluded ${c.excluded_request_count === 1 ? 'request is' : 'requests are'} included in the missing count.`)}</p>}
      <p>{t('比较假设主力使用相同的输入和缓存命中量，未上报的缓存写入量介于零和未命中输入量之间；主力实际输出和缓存行为可能不同。', 'The comparison assumes the same input and cache reads, with unreported cache writes between zero and uncached input; actual Primary output and caching can differ.')}</p>
      <p>{t('参考当前来源的 API 价格，不是历史账单价格。', 'The basis is the current source API reference price, not a historical bill price.')}</p>
    </Disclosure>
  </section>;
}
