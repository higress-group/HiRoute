import { useEffect, useRef, useState } from 'react';
import { observationRead } from '../observation-client';
import { selectionReasonLabel } from '../selection-reasons';
import type { TrustReplay } from './types';

export type ReplayScope = { planId: string; revision: number; branchId: string; savedThreshold: number; active: boolean };
type ReplayCache = { key: string; window: { from_ms: number; to_ms: number }; entries: Map<number, TrustReplay> };
export function TrustReplayPreview({ scope, threshold, language }: { scope: ReplayScope; threshold: number; language: 'zh' | 'en' }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [result, setResult] = useState<ReplayCache | null>(null);
  const cache = useRef<ReplayCache | null>(null);
  const [failed, setFailed] = useState('');
  const [retry, setRetry] = useState(0);
  const [visible, setVisible] = useState(document.visibilityState === 'visible');
  const scopeKey = JSON.stringify([scope.planId, scope.revision, scope.branchId, scope.savedThreshold]);
  const key = JSON.stringify([scopeKey, threshold]);
  const latest = useRef({ scopeKey, threshold, active: scope.active });
  latest.current = { scopeKey, threshold, active: scope.active };
  const data = scope.active && visible && result?.key === scopeKey ? result.entries.get(threshold) : undefined;
  const loadedScope = result?.key === scopeKey;
  const cachedThreshold = Boolean(data);
  useEffect(() => { const update = () => setVisible(document.visibilityState === 'visible'); document.addEventListener('visibilitychange', update); return () => document.removeEventListener('visibilitychange', update); }, []);
  // One window belongs to the scope, even if a hidden or failed read is retried.
  useEffect(() => {
    if (!scope.active || !visible) return;
    if (cache.current?.key !== scopeKey) {
      const to_ms = Date.now();
      cache.current = { key: scopeKey, window: { from_ms: to_ms - 7 * 86_400_000, to_ms }, entries: new Map() };
    }
    if (cache.current.entries.has(scope.savedThreshold)) return;
    if (!Number.isInteger(scope.savedThreshold) || scope.savedThreshold < 0 || scope.savedThreshold > 1000) return;
    let current = true;
    const abort = new AbortController();
    const evidenceWindow = cache.current.window;
    setFailed('');
    observationRead<TrustReplay>('plan_trust_replay', { plan_id: scope.planId, revision: scope.revision, branch_id: scope.branchId, ...evidenceWindow, thresholds_millis: [scope.savedThreshold] }, abort.signal)
      .then(data => {
        if (!current || !latest.current.active || latest.current.scopeKey !== scopeKey || document.visibilityState !== 'visible' || cache.current?.key !== scopeKey) return;
        const next = { ...cache.current, entries: new Map([[scope.savedThreshold, data]]) };
        cache.current = next; setResult(next);
      })
      .catch(() => { if (current && latest.current.active && latest.current.scopeKey === scopeKey && document.visibilityState === 'visible') setFailed(scopeKey); });
    return () => { current = false; abort.abort(); };
  }, [scopeKey, scope.planId, scope.revision, scope.branchId, scope.savedThreshold, scope.active, retry, visible]);
  useEffect(() => {
    if (!scope.active || !visible || !loadedScope || cachedThreshold || !Number.isInteger(threshold) || threshold < 0 || threshold > 1000) return;
    let current = true;
    const abort = new AbortController();
    setFailed('');
    const timer = window.setTimeout(() => {
      const previous = cache.current;
      if (previous?.key !== scopeKey) return;
      observationRead<TrustReplay>('plan_trust_replay', { plan_id: scope.planId, revision: scope.revision, branch_id: scope.branchId, ...previous.window, thresholds_millis: [threshold] }, abort.signal)
        .then(data => {
          if (!current || !latest.current.active || latest.current.scopeKey !== scopeKey || latest.current.threshold !== threshold || document.visibilityState !== 'visible' || cache.current?.key !== scopeKey) return;
          const entries = new Map(cache.current.entries);
          entries.set(threshold, data);
          while (entries.size > 32) {
            const oldest = [...entries.keys()].find(value => value !== scope.savedThreshold);
            if (oldest === undefined) break;
            entries.delete(oldest);
          }
          const next = { ...cache.current, entries };
          cache.current = next; setResult(next);
        })
        .catch(() => { if (current && latest.current.active && latest.current.scopeKey === scopeKey && latest.current.threshold === threshold && document.visibilityState === 'visible') setFailed(key); });
    }, 300);
    return () => { current = false; abort.abort(); window.clearTimeout(timer); };
  }, [scopeKey, key, scope.planId, scope.revision, scope.branchId, scope.savedThreshold, scope.active, threshold, loadedScope, cachedThreshold, retry, visible]);
  const row = data?.thresholds.find(r => r.threshold_millis === threshold);
  const share = (n: number | null | undefined) => n == null ? '—' : `${Math.round(n * 100)}%`;
  const bar = (n: number | null | undefined) => Math.max(0, Math.min(100, (n ?? 0) * 100));
  const excluded = Object.entries(data?.excluded_by_reason ?? {}).filter(([, count]) => count > 0);
  const excludedTotal = excluded.reduce((sum, [, count]) => sum + count, 0);
  return <div className="trust-replay">
    <div className="trust-replay-head"><strong>{t('用你的记录回放此设置', 'Replay this setting on your records')}</strong><span>{t(`当前发布的版本 · 最近 7 天${data ? ` · ${data.included + excludedTotal} 个阶段` : ''}`, `Active version · last 7 days${data ? ` · ${data.included + excludedTotal} stages` : ''}`)}</span></div>
    {failed === key || failed === scopeKey ? <p className="trust-replay-state" role="alert">{t('暂时无法读取回放。', 'Replay could not be read.')} <button type="button" className="btn btn-quiet" onClick={() => setRetry(n => n + 1)}>{t('重试', 'Retry')}</button></p>
      : !data || !row ? <p className="trust-replay-state">{t('正在读取回放…', 'Reading replay…')}</p>
      : data.included === 0 ? <p className="trust-replay-state">{t('没有可用于回放的记录，不估算占比。', 'No usable recorded evidence; shares are not estimated.')}</p>
      : <>
        <div className="trust-replay-bar" role="img" aria-label={t(`阈值 ${threshold / 1000}：常规 ${share(row.regular_share)}，主力 ${share(row.primary_share)}`, `At ${threshold / 1000}: Regular ${share(row.regular_share)}, Primary ${share(row.primary_share)}`)}><svg viewBox="0 0 100 14" preserveAspectRatio="none" aria-hidden="true"><rect className="regular" width={bar(row.regular_share)} height="14" /><rect className="primary" x={bar(row.regular_share)} width={bar(row.primary_share)} height="14" /></svg></div>
        <div className="trust-replay-legend">
          <span><i className="regular" />{t('常规', 'Regular')} <strong>{share(row.regular_share)}</strong> <span>{t(`（${row.regular} 个）`, `(${row.regular})`)}</span></span>
          <span><i className="primary" />{t('主力', 'Primary')} <strong>{share(row.primary_share)}</strong> <span>{t(`（${row.primary} 个）`, `(${row.primary})`)}</span></span>
          <span className="muted">{t(`当前设置实际 ${share(data.recorded_share_at_current)} 交给常规`, `Recorded at the current setting: ${share(data.recorded_share_at_current)} Regular`)}</span>
        </div>
      </>}
    {data && row && data.included > 0 && <p className="field-help">{[
      t(`阈值 ${threshold / 1000}：计入 ${data.included} 个阶段，其中 ${row.protected_as_primary} 个低分保护按主力计算`, `At ${threshold / 1000}: ${data.included} stages included; ${row.protected_as_primary} low-score protections count as Primary`),
      excluded.length ? t(`未计入 ${excludedTotal} 个：${excluded.map(([reason, count]) => `${selectionReasonLabel(reason, language)} ${count}`).join('、')}`, `${excludedTotal} not included: ${excluded.map(([reason, count]) => `${selectionReasonLabel(reason, language)} ${count}`).join(', ')}`) : '',
      row.newly_regular > 0 ? t(`${row.newly_regular} 个阶段会首次交给常规模型，之前由主力处理，没有常规模型的胜任证据`, `${row.newly_regular} stages would newly go to Regular; Primary handled them, so they carry no Regular competence evidence`) : '',
    ].filter(Boolean).join(t('；', '. '))}{t('。这是基于历史分配的估算，不是预测。', '. This estimates historical allocation; it is not a prediction.')}</p>}
  </div>;
}
