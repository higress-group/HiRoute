import type { ReactNode } from 'react';
import { useEffect, useRef } from 'react';
import { UiIcon } from '../ui';

export type SafeFact = {
  event_id: string; sequence: number; occurred_at_ms: number | null;
  event_kind: string; attempt_ordinal: number | null; native_model: string | null;
  input_tokens: number | null; output_tokens: number | null;
  cache_read_tokens: number | null; cache_write_tokens: number | null;
  reasoning_tokens: number | null; outcome: string | null; sensitive_fields_deleted: boolean;
};

export function RuntimeFacts({ language, summary, facts, partial, busy, error, next, contentState = 'recorded', usage, onNext, onClose, onClear }: {
  language: 'zh' | 'en'; facts: SafeFact[]; partial: boolean; busy: boolean; error: string;
  summary?: { outcome: string | null; finalModel: string | null; modelFallback: boolean | null };
  contentState?: 'recorded' | 'partial' | 'cleared';
  usage?: ReactNode;
  next: boolean; onNext(): void; onClose(): void; onClear?(): void;
}) {
  const text = (cn: string, en: string) => language === 'zh' ? cn : en;
  const meaningful = facts.filter(fact => fact.native_model !== null || fact.outcome !== null || [fact.input_tokens, fact.output_tokens, fact.cache_read_tokens, fact.cache_write_tokens, fact.reasoning_tokens].some(value => value !== null));
  const outcomes: Record<string, string> = { accepted: text('响应已交付', 'Response delivered'), failed: text('失败', 'Failed'), cancelled: text('已取消', 'Cancelled'), completed: text('请求已完成', 'Request completed') };
  const finalModel = summary?.finalModel ?? [...meaningful].reverse().find(fact => fact.native_model)?.native_model ?? null;
  const panel = useRef<HTMLElement>(null);
  const returnTarget = useRef<HTMLElement | null>(null);

  useEffect(() => {
    returnTarget.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const backgrounds: { element: HTMLElement; inert: boolean }[] = [];
    let branch = panel.current?.parentElement;
    while (branch?.parentElement && branch !== document.body) {
      for (const sibling of Array.from(branch.parentElement.children)) {
        if (sibling !== branch && sibling instanceof HTMLElement) {
          backgrounds.push({ element: sibling, inert: sibling.inert });
          sibling.inert = true;
        }
      }
      branch = branch.parentElement;
    }
    const frame = requestAnimationFrame(() => panel.current?.focus());
    return () => {
      cancelAnimationFrame(frame);
      backgrounds.forEach(({ element, inert }) => { element.inert = inert; });
      requestAnimationFrame(() => returnTarget.current?.isConnected && returnTarget.current.focus());
    };
  }, []);

  const contentLabels = {
    recorded: text('已记录', 'Recorded'),
    partial: text('部分缺失', 'Partial'),
    cleared: text('已清理', 'Cleared'),
  };

  return <aside ref={panel} className="run-inspector" role="dialog" aria-modal="true" aria-labelledby="run-inspector-title" tabIndex={-1} data-autofocus onKeyDown={event => {
    if (event.key === 'Escape') { event.preventDefault(); onClose(); }
    if (event.key !== 'Tab') return;
    const controls = Array.from(panel.current?.querySelectorAll<HTMLElement>('button:not([disabled]), summary, [tabindex]:not([tabindex="-1"])') ?? []).filter(element => element.getClientRects().length > 0);
    if (!controls.length) return;
    if (event.shiftKey && document.activeElement === controls[0]) { event.preventDefault(); controls.at(-1)?.focus(); }
    else if (!event.shiftKey && document.activeElement === controls.at(-1)) { event.preventDefault(); controls[0].focus(); }
  }}>
    <header className="inspector-head"><div><h3 id="run-inspector-title">{text('运行记录', 'Run record')}</h3><p>{text('仅展示已记录的信息', 'Recorded information only')}</p></div><button className="icon-btn" type="button" aria-label={text('关闭运行记录', 'Close run record')} onClick={onClose}><UiIcon name="close" /></button></header>
    <div className="inspector-body">
      {busy && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('正在读取运行记录…', 'Reading the run record…')}</p></div>}
      {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><div><strong>{text('运行记录暂不可用', 'Run record unavailable')}</strong><p>{text('会话正文仍可阅读。', 'Conversation content remains available.')}</p><button className="btn" type="button" onClick={onNext}>{text('重试', 'Retry')}</button></div></div>}
      {partial && <div className="callout warn" role="status"><UiIcon name="warning" /><span>{text('部分运行记录尚未取得，当前内容可能不完整。', 'Some run records are unavailable, so this view may be incomplete.')}</span></div>}
      <section className="fact-section"><h4>{text('模型使用', 'Model usage')}</h4><dl className="fact-kv"><dt>{text('最终模型', 'Final model')}</dt><dd>{finalModel ?? text('尚未记录', 'Not recorded')}</dd><dt>{text('请求内回退', 'Request fallback')}</dt><dd>{summary?.modelFallback === true ? text('已发生', 'Observed') : summary?.modelFallback === false ? text('未发生', 'None') : text('尚未记录', 'Not recorded')}</dd><dt>{text('结果', 'Result')}</dt><dd>{summary?.outcome ? outcomes[summary.outcome] ?? text('结果已记录', 'Result recorded') : text('尚未记录', 'Not recorded')}</dd></dl></section>
      {usage}
      <section className="fact-section"><h4>{text('记录状态', 'Record status')}</h4><dl className="fact-kv"><dt>{text('会话正文', 'Session content')}</dt><dd>{contentLabels[contentState]}</dd></dl></section>
      {next && <button className="btn" type="button" disabled={busy} onClick={onNext}>{text('下一页记录', 'Next records')}</button>}
      {onClear && <button className="btn inspector-clear" type="button" onClick={onClear}><UiIcon name="trash" />{text('清理此会话', 'Clear this session')}</button>}
    </div>
  </aside>;
}
