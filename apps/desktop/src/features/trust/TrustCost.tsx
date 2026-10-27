import { formatMoney } from '../../ui';

export type TrustCostValue = { kind: 'unavailable'; reason: string } | {
  kind: 'available'; difference_micros: number | null;
  comparison: { currency: string; coverage: 'complete' | 'partial' | 'unknown'; request_count: number; compared_request_count: number; missing_request_count: number; excluded_request_count: number; routed_estimate_micros: number | null; reference_estimate_micros: number | null; same_usage_bounds?: { reference_min_micros: number; reference_max_micros: number; difference_min_micros: number; difference_max_micros: number } | null; compared_routed_subtotal_micros: number | null; compared_reference_subtotal_micros: number | null; retention_boundary_partial: boolean; arithmetic_overflow: boolean };
};

export function TrustCost({ value, language }: { value: TrustCostValue; language: 'zh' | 'en' }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const title = <h3>{t('费用对比（估算）', 'Cost comparison (estimate)')}</h3>;
  if (value.kind === 'unavailable') {
    const reason = value.reason === 'primary_not_unique' ? t('需要唯一的已发布主力模型。', 'A unique published Primary model is required.')
      : value.reason === 'currency_not_unique' ? t('需要唯一的可用 API 参考币种。', 'One available API-reference currency is required.')
      : value.reason === 'prices_unavailable' || value.reason === 'reference_unavailable' ? t('当前来源的 API 参考价格不可用。', 'Current source-specific API reference prices are unavailable.')
      : t('记录正在准备或证据不足，请稍后重试。', 'Records are catching up or evidence is unavailable. Retry later.');
    return <section className="trust-card trust-cost" aria-label={t('常规组成本对比', 'Regular cost comparison')}>{title}<p className="field-help">{t('暂不可用：', 'Unavailable: ')}{reason}</p></section>;
  }
  const c = value.comparison;
  const amount = (micros: number | null) => micros === null ? t('未知', 'Unknown') : formatMoney(c.currency, (micros / 1_000_000).toFixed(4));
  const signedAmount = (micros: number) => `${micros > 0 ? '+' : micros < 0 ? '−' : ''}${amount(Math.abs(micros))}`;
  const difference = value.difference_micros;
  return <section className="trust-card trust-cost" aria-label={t('常规组成本对比', 'Regular cost comparison')}>
    {title}
    <div className="trust-cost-row"><span>{t('常规模型阶段的估算费用', 'Estimated cost of Regular stages')}</span><strong>{amount(c.routed_estimate_micros)}</strong></div>
    <div className="trust-cost-row"><span>{t('同样用量按主力的 API 等价价格', 'Same usage at Primary API-equivalent prices')}</span><strong>{amount(c.reference_estimate_micros)}</strong></div>
    {difference === null ? <p className="trust-cost-row">{t('完整范围的差额未知。', 'The full-scope difference is unknown.')}</p>
      : <div className="trust-cost-row"><span>{difference > 0 ? t('额外成本', 'Additional cost') : t('估算差额', 'Estimated difference')}</span><strong className={difference > 0 ? 'more' : difference < 0 ? 'less' : ''}>{difference > 0 ? '+' : difference < 0 ? '−' : ''}{amount(Math.abs(difference))}</strong></div>}
    {c.same_usage_bounds && <>
      <div className="trust-cost-row"><span>{t('主力参考费用范围', 'Primary reference cost range')}</span><strong>{amount(c.same_usage_bounds.reference_min_micros)} – {amount(c.same_usage_bounds.reference_max_micros)}</strong></div>
      <div className="trust-cost-row"><span>{t('估算差额范围', 'Estimated difference range')}</span><strong>{signedAmount(c.same_usage_bounds.difference_min_micros)} – {signedAmount(c.same_usage_bounds.difference_max_micros)}</strong></div>
      <p className="field-help">{t('范围假设主力使用相同的输入和缓存命中量；未上报的缓存写入量介于零和未命中输入量之间。', 'The range assumes the same input and cache reads for Primary; unreported cache writes range from zero to uncached input.')}</p>
    </>}
    {c.coverage !== 'complete' && c.compared_request_count > 0 && <div className="trust-cost-row"><span>{t('仅完整配对请求的小计', 'Paired complete-request subtotals')}</span><strong>{amount(c.compared_routed_subtotal_micros)} / {amount(c.compared_reference_subtotal_micros)}</strong></div>}
    <p className="field-help">{t(`可完整比较 ${c.compared_request_count}/${c.request_count} 个请求 · 缺失 ${c.missing_request_count} · 排除 ${c.excluded_request_count}。`, `Fully comparable: ${c.compared_request_count}/${c.request_count} requests · ${c.missing_request_count} missing · ${c.excluded_request_count} excluded.`)}{c.missing_request_count > 0 && t('归属不明的请求保守计入缺口，排除数量包含在缺失数量中。', ' Requests with unknown attribution count as gaps. Excluded gaps are included in the missing count.')}</p>
    {(c.retention_boundary_partial || c.arithmetic_overflow) && <p className="field-help">{c.arithmetic_overflow ? t('金额超出计算范围。', 'An amount exceeds the arithmetic range.') : t('部分记录已超出保留期限。', 'Some records are outside retention.')}</p>}
    <p className="field-help">{t('按所选版本、分类和常规组筛选，沿用已记录 token，含重试、不含决策模型调用。主力实际会产生不同的 token 和缓存，这里只按同样用量换算；不是账单，也不是实际节省。', 'Scoped to the selected version, category and Regular group, using recorded tokens with retries and without decision-model calls. Primary would produce different tokens and caching; this converts the same usage only. It is not a bill or a cash saving.')}</p>
  </section>;
}
