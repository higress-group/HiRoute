import { invoke } from '../../host/index.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Dialog, UiIcon } from '../../ui';
import { confirmDiscard } from '../../ui/discard-guard';
import type { Language, PriceContext, PriceDisplay, PriceOutcome } from './types';

const emptyRates = ['', '', '', ''] as const;

function safeErrorCode(error: unknown): string {
  const pending: unknown[] = [error];
  const visited = new Set<object>();
  while (pending.length && visited.size < 8) {
    const current = pending.shift();
    if (!current || typeof current !== 'object' || visited.has(current)) continue;
    visited.add(current);
    const value = current as Record<string, unknown>;
    if (typeof value.code === 'string' && /^[A-Za-z0-9_.\/-]{1,80}$/.test(value.code)) return value.code;
    pending.push(value.error, value.envelope, value.failure);
  }
  return 'PRICE_CHANGE_FAILED';
}

function errorMessage(code: string, zh: boolean): string {
  if (code === 'INVALID_PRICE_DECIMAL') return zh ? '请输入非负价格，最多保留六位小数。' : 'Enter non-negative prices with up to six decimal places.';
  if (code === 'REVISION_CONFLICT') return zh ? '价格已在别处更新。刷新当前价格后再试，输入已保留。' : 'Pricing changed elsewhere. Refresh it and try again; your input is retained.';
  if (code === 'PRICE_CONTEXT_UNAVAILABLE') return zh ? '暂时无法取得保存所需的价格版本，请刷新模型后再试。' : 'The pricing version required to save is unavailable. Refresh the model and try again.';
  return zh ? '价格未保存。请读取当前状态后重试，输入已保留。' : 'Pricing was not saved. Refresh the current state and try again; your input is retained.';
}

/** Exact V3 pricing dialog backed by effective-price and price-override operations. */
export function PriceEditor({
  modelName,
  sourceName,
  contexts,
  language,
  writable,
  onRefresh,
  onOperation,
  onClose,
  onSaved,
}: {
  modelName: string;
  sourceName: string;
  contexts: PriceContext[];
  language: Language;
  writable: boolean;
  onRefresh: () => Promise<void>;
  onOperation: (outcome: PriceOutcome) => void;
  onClose: () => void;
  onSaved: () => void;
}) {
  const zh = language === 'zh';
  const [contextIndex, setContextIndex] = useState(0);
  const context = contexts[contextIndex] ?? contexts[0];
  const [mode, setMode] = useState<'catalog' | 'manual'>('catalog');
  const [rates, setRates] = useState<string[]>([...emptyRates]);
  const [display, setDisplay] = useState<PriceDisplay | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const dirty = useRef(false);
  const generation = useRef(0);
  const target = useMemo(() => context ? JSON.stringify({
    target_locator: context.target_locator,
    currency: context.currency,
    valuation_kind: context.valuation_kind,
  }) : '', [context]);

  async function read() {
    if (!context) return;
    const current = ++generation.current;
    setLoading(true);
    setError('');
    try {
      const result = await invoke<PriceDisplay>('effective_price_query', {
        input: { targets: [{ query_id: 'selected', ...JSON.parse(target) }] },
      });
      if (generation.current !== current) return;
      setDisplay(result);
      if (!dirty.current) {
        setRates(result.display_rates[0]?.map(value => value ?? '') ?? [...emptyRates]);
        setMode(result.result.items[0]?.quote.origin === 'manual' ? 'manual' : 'catalog');
      }
    } catch (cause) {
      if (generation.current === current) setError(safeErrorCode(cause));
    } finally {
      if (generation.current === current) setLoading(false);
    }
  }

  useEffect(() => {
    dirty.current = false;
    setRates([...emptyRates]);
    setDisplay(null);
    setMode('catalog');
    setNotice('');
    void read();
    return () => { generation.current += 1; };
  }, [target]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [notice]);

  function requestClose() {
    if (busy) return;
    onClose();
  }

  async function chooseContext(nextIndex: number) {
    if (busy || nextIndex === contextIndex) return;
    if (dirty.current && !(await confirmDiscard(language))) return;
    setContextIndex(nextIndex);
  }

  async function submit(form?: HTMLFormElement) {
    if (busy || !writable || !context) return;
    const editContext = display?.result.items[0]?.edit_context;
    if (!editContext) {
      setError('PRICE_CONTEXT_UNAVAILABLE');
      return;
    }
    const formValues = form ? new FormData(form) : null;
    const submittedRates = mode === 'manual' && formValues
      ? [String(formValues.get('input_rate') ?? ''), String(formValues.get('output_rate') ?? ''), rates[2] ?? '', rates[3] ?? '']
      : rates;
    if (mode === 'manual' && (!submittedRates[0] || !submittedRates[1] || !/^\d+(?:\.\d{1,6})?$/.test(submittedRates[0]) || !/^\d+(?:\.\d{1,6})?$/.test(submittedRates[1]))) {
      setRates(submittedRates);
      setError('INVALID_PRICE_DECIMAL');
      return;
    }
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const action = mode === 'catalog'
        ? { kind: 'follow_catalog' as const }
        : {
            kind: 'set' as const,
            input_uncached: submittedRates[0],
            output: submittedRates[1],
            // The compact V3 form edits input/output only. Preserve effective cache
            // rates instead of silently replacing them with zeroes.
            cache_read: submittedRates[2] || null,
            cache_write: submittedRates[3] || null,
          };
      const outcome = await invoke<PriceOutcome>('preview_price_change', { input: { ...context, ...editContext, language, action } });
      onOperation(outcome);
      if (outcome.state === 'cancelled_before_apply') {
        setNotice(zh ? '已取消，价格没有改变。' : 'Cancelled. Pricing was not changed.');
      } else if (outcome.operation?.state === 'succeeded') {
        await onRefresh();
        dirty.current = false;
        onSaved();
      } else {
        setNotice(zh ? '正在确认保存结果，输入会保留。' : 'Confirming the save result. Your input is retained.');
      }
    } catch (cause) {
      setError(safeErrorCode(cause));
    } finally {
      setBusy(false);
    }
  }

  const origin = display?.result.items[0]?.quote.origin;
  const visibleRates = display?.display_rates[0];
  const trimRate = (value: string | null | undefined) => value
    ? value.replace(/(?:\.0+|(?:(\.\d*?)0+))$/, '$1')
    : '—';
  const catalogSummary = origin && origin !== 'manual' && visibleRates?.[0] && visibleRates?.[1]
    ? `${context?.currency === 'CNY' ? '¥' : context?.currency === 'USD' ? '$' : `${context?.currency ?? ''} `}${trimRate(visibleRates[0])} / ${context?.currency === 'CNY' ? '¥' : context?.currency === 'USD' ? '$' : `${context?.currency ?? ''} `}${trimRate(visibleRates[1])}`
    : zh ? '目录暂无价格时显示为未计价' : 'Usage remains unpriced when no catalog price exists';
  return <Dialog
    open
    title={zh ? '调整价格' : 'Edit pricing'}
    description={modelName}
    closeLabel={zh ? '关闭价格设置' : 'Close pricing'}
    closeDisabled={busy}
    onClose={requestClose}
    footer={<><button className="btn" type="button" disabled={busy} onClick={requestClose}>{zh ? '取消' : 'Cancel'}</button><button className="btn btn-primary" type="submit" form="price-editor-form" disabled={busy || loading || !writable || !context || !display?.result.items[0]?.edit_context}>{busy ? (zh ? '正在保存…' : 'Saving…') : (zh ? '保存价格' : 'Save pricing')}</button></>}
  >
    <form id="price-editor-form" className="price-editor" noValidate onSubmit={event => { event.preventDefault(); void submit(event.currentTarget); }}>
      <div className="option-row"><div><strong>{zh ? '费率设置' : 'Pricing'}</strong><span>{zh ? '仅用于估算，不改变接入权益或计费类别。' : 'Used for estimates; does not change access or billing classification.'}</span></div></div>
      {contexts.length > 1 && <label className="field"><span className="field-label">{zh ? '报价口径' : 'Price context'}</span><select className="select" value={contextIndex} disabled={busy} onChange={event => void chooseContext(Number(event.target.value))}>{contexts.map((item, index) => <option value={index} key={`${item.currency}/${item.valuation_kind}`}>{item.currency} · {item.valuation_kind === 'usage_estimate' ? (zh ? '用量估算' : 'Usage estimate') : (zh ? 'API 等价值' : 'API equivalent')}</option>)}</select></label>}
      {loading ? <div className="oc-status-row" role="status"><span className="oc-spinner" /><div className="row-main"><strong>{zh ? '正在读取当前价格' : 'Reading current pricing'}</strong><p>{sourceName}</p></div></div> : <div className="price-mode-list">
        <label className="check-row"><input type="radio" name="price-mode" checked={mode === 'catalog'} disabled={busy} onChange={() => { dirty.current = true; setMode('catalog'); setError(''); }} /><div><strong>{zh ? '跟随目录价格' : 'Follow catalog pricing'}</strong><span>{catalogSummary}</span></div></label>
        <label className="check-row"><input type="radio" name="price-mode" checked={mode === 'manual'} disabled={busy} onChange={() => { dirty.current = true; setMode('manual'); setError(''); }} /><div><strong>{zh ? '手工设置费率' : 'Set rates manually'}</strong><span>{context ? `${zh ? '币种' : 'Currency'} ${context.currency} · ${zh ? '每 100 万 tokens' : 'per million tokens'}` : '—'}</span></div></label>
      </div>}
      {!loading && mode === 'manual' && <div className="oc-field-grid price-rate-fields">
        {[zh ? '输入价格' : 'Input price', zh ? '输出价格' : 'Output price'].map((label, index) => <label className="field" key={label}><span className="field-label">{label}</span><input className="input" name={index === 0 ? 'input_rate' : 'output_rate'} data-autofocus={index === 0 ? true : undefined} inputMode="decimal" required pattern="[0-9]+(\.[0-9]{1,6})?" value={rates[index]} placeholder={`${context?.currency ?? ''} / 1M tokens`} onChange={event => { dirty.current = true; setError(''); setRates(current => current.map((value, item) => item === index ? event.target.value : value)); }} /></label>)}
      </div>}
      {!writable && <div className="callout warn"><UiIcon name="warning" /><span>{zh ? '当前接入不支持价格编辑。' : 'This connection does not support pricing edits.'}</span></div>}
      {notice && <div className="callout" role="status"><UiIcon name="info" /><span>{notice}</span></div>}
      {error && <div className="callout bad" role="alert" data-error-code={error}><UiIcon name="warning" /><span>{errorMessage(error, zh)}</span></div>}
    </form>
  </Dialog>;
}
