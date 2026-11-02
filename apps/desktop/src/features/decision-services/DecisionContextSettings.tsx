import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Disclosure } from '../../ui';
import type { DecisionBundle } from './form-state';
import type { DecisionServiceView, ResolvedDecisionContext } from './contracts';
import { contextDraft, contextErrors, contextPolicy, contextSignature, typesafeDestination, type DecisionContextDraft } from './context-state';

export type ContextPreview = { signature: string; state: 'loading' | 'invalid' | 'error' }
  | { signature: string; state: 'ready'; resolved: ResolvedDecisionContext };

export function DecisionContextSettings({ bundle, view, language, refreshKey, onChange, onPreview }: {
  bundle: DecisionBundle; view: DecisionServiceView | null; language: 'zh' | 'en'; refreshKey: number;
  onChange(context: DecisionContextDraft): void; onPreview(preview: ContextPreview): void;
}) {
  const zh = language === 'zh', t = (cn: string, en: string) => zh ? cn : en;
  const draft = contextDraft(bundle), policy = contextPolicy(bundle), signature = contextSignature(bundle);
  const errors = contextErrors(bundle, language);
  const [preview, setPreview] = useState<ContextPreview>({ signature, state: 'loading' });
  const [retry, setRetry] = useState(0);
  const [questionOpen, setQuestionOpen] = useState(false);
  const questionInput = useRef<HTMLInputElement>(null);
  const id = useId();
  useEffect(() => { if (errors.context_question) setQuestionOpen(true); }, [errors.context_question]);
  const callback = useRef(onPreview); callback.current = onPreview;
  useEffect(() => {
    let active = true;
    function update(value: ContextPreview) { if (active) { setPreview(value); callback.current(value); } }
    if (!policy || !bundle.model.trim() || !bundle.endpoint.trim() || !bundle.wire_protocol) {
      update({ signature, state: 'invalid' }); return;
    }
    update({ signature, state: 'loading' });
    const timer = window.setTimeout(() => {
      void invoke<ResolvedDecisionContext>('preview_decision_context', { input: {
        endpoint: bundle.endpoint.trim(), model: bundle.model.trim(), wire_protocol: bundle.wire_protocol, policy,
      } }).then(resolved => update({ signature, state: 'ready', resolved }))
        .catch(() => update({ signature, state: 'error' }));
    }, 200);
    return () => { active = false; window.clearTimeout(timer); };
  }, [signature, retry, refreshKey]);
  const current = preview.signature === signature && preview.state === 'ready' ? preview.resolved : null;
  const source = (value: ResolvedDecisionContext['request_source']) => value === 'user' ? t('自定义', 'Custom')
    : value === 'fallback' ? t('未知窗口 · 保守默认', 'Unknown window · conservative default') : t('模型元信息', 'Model metadata');
  const saved = view?.service && 'context' in view.service ? view.service.context?.resolved : null;
  const changed = current && saved && current.context_digest !== saved.context_digest;
  return <section className="decision-context" aria-label={t('上下文窗口', 'Context window')}>
    <div className="decision-context-heading"><h3>{t('上下文窗口', 'Context window')}</h3>
      <div className="segmented" role="group" aria-label={t('窗口设置', 'Window setting')}>
        {(['automatic', 'custom'] as const).map(mode => <button key={mode} type="button" className={'segment' + (draft.mode === mode ? ' active' : '')} aria-pressed={draft.mode === mode} onClick={() => onChange({ ...draft, mode })}>{mode === 'automatic' ? t('自动', 'Automatic') : t('自定义', 'Custom')}</button>)}
      </div>
    </div>
    {draft.mode === 'custom' && <label className="field"><span className="field-label">{t('总窗口（tokens）', 'Request window (tokens)')}</span>
      <input className="input" data-native-field="context_window" inputMode="numeric" aria-invalid={!!errors.context_window} aria-describedby={errors.context_window ? `${id}-request-error` : undefined} value={draft.window} onChange={event => onChange({ ...draft, window: event.target.value })} />
      {errors.context_window && <span className="field-error" role="alert" id={`${id}-request-error`}>{errors.context_window}</span>}
    </label>}
    {errors.context_question && <div className="decision-context-validation" role="alert"><p>{errors.context_question}</p>
      <button className="btn btn-quiet" type="button" onClick={() => {
        setQuestionOpen(true);
        requestAnimationFrame(() => {
          questionInput.current?.focus({ preventScroll: true });
          questionInput.current?.closest('.field')?.scrollIntoView({ block: 'center', behavior: 'instant' });
        });
      }}>{t('检查问题上限', 'Review question limit')}</button>
    </div>}
    {!Object.keys(errors).length && <div className="decision-context-effective" role="status" aria-live="polite">
      {current ? <>
        <dl className="decision-context-summary"><div><dt>{t('有效总窗口', 'Effective request window')}</dt><dd>{current.request_window_tokens.toLocaleString()} <small>{source(current.request_source)}</small></dd></div>
          <div><dt>{t('每个问题上限', 'Per-question limit')}</dt><dd>{current.per_question_window_tokens.toLocaleString()} <small>{source(current.per_question_source)}</small></dd></div></dl>
        {(current.request_source === 'metadata' || current.per_question_source === 'metadata') && current.metadata?.basis === 'maintainer_policy' && <span className="field-help decision-context-basis">{t('自动窗口采用已维护的产品策略值。', 'Automatic limits use a maintained product policy value.')}</span>}
        {(current.request_source === 'metadata' || current.per_question_source === 'metadata') && current.metadata?.basis === 'inferred' && <span className="field-help decision-context-basis">{t('元信息窗口采用有来源记录的折算值。', 'Metadata limits use a source-backed normalization.')}</span>}
      </> : <span className="field-help">{preview.state === 'error' ? t('窗口暂时无法解析，请重试后保存。', 'The window could not be resolved. Retry before saving.')
        : preview.state === 'invalid' ? t('填写有效的模型、地址和窗口后显示解析结果。', 'Enter a valid model, URL and window to see the resolved limits.') : t('正在解析窗口…', 'Resolving window…')}</span>}
      <button className="btn btn-quiet" type="button" disabled={preview.state === 'loading' || preview.state === 'invalid'} onClick={() => setRetry(value => value + 1)}>{t('刷新窗口', 'Refresh window')}</button>
    </div>}
    <p className="field-help">{t('保留当前输入和决策问题，剩余空间用于历史。窗口按保守估算分配，不代表精确 token 用量。', 'Current input and decision questions take priority; history uses the remaining space. Limits use a conservative estimate, not an exact token count.')}</p>
    {view && !saved && <p className="field-help">{t('已保存版本沿用原有预算；保存新配置后才会采用以上窗口。', 'The saved revision retains its original budget. Save a new configuration to adopt these windows.')}</p>}
    {changed && <p className="field-help">{t(`已保存总窗口 ${saved.request_window_tokens.toLocaleString()}；保存后会采用当前解析结果，需在路由中重新选用并发布。`, `Saved request window: ${saved.request_window_tokens.toLocaleString()}. Saving adopts the current result; select and publish it in the route.`)}</p>}
    {typesafeDestination(bundle) && <label className="decision-context-toggle"><input type="checkbox" checked={draft.typesafe_long_context === 'automatic'} onChange={event => onChange({ ...draft, typesafe_long_context: event.target.checked ? 'automatic' : 'disabled' })} /><span>{t('启用 TypeSafe 长上下文', 'Enable TypeSafe long context')}<small className="field-help">{t('关闭后解析供应商的较小窗口；已发布的路由保持原配置。', 'Turning this off resolves the smaller provider windows. Published routes retain their saved configuration.')}</small></span></label>}
    <Disclosure label={t('高级：单个问题上限', 'Advanced: per-question limit')} language={language} open={questionOpen} onOpenChange={setQuestionOpen}>
      <label className="field"><span className="field-label">{t('自定义问题上限（tokens）', 'Custom question limit (tokens)')}</span>
        <input ref={questionInput} className="input" data-native-field="context_question" inputMode="numeric" disabled={draft.mode !== 'custom'} aria-invalid={!!errors.context_question} aria-describedby={`${id}-question-help${errors.context_question ? ` ${id}-question-error` : ''}`} value={draft.question} placeholder={t('留空以跟随元信息或总窗口', 'Follow metadata or request window')} onChange={event => onChange({ ...draft, question: event.target.value })} />
        {errors.context_question && <span className="field-error" id={`${id}-question-error`}>{errors.context_question}</span>}
        <span className="field-help" id={`${id}-question-help`}>{t('自定义总窗口不会取消独立的问题上限。切换自定义模式后，可在此覆盖。', 'A custom request window preserves an independent question limit. Custom mode enables an explicit override here.')}</span>
      </label>
    </Disclosure>
  </section>;
}
