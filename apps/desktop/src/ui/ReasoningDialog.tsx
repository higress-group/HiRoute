import { useState } from 'react';
import { Dialog } from './Dialog';
import { UiIcon } from './UiIcon';
import type { Selection } from '../plan-editor';

export type NativeReasoning =
  | { kind: 'fixed'; profile: string }
  | { kind: 'toggle'; parameter: string }
  | { kind: 'discrete'; parameter: string; profiles: string[]; default_profile?: string }
  | { kind: 'budget'; parameter: string; minimum_tokens: number; maximum_tokens: number; step_tokens: number };

/** Apply commits to the route draft. Escape, backdrop and Cancel discard local edits. */
export function ReasoningDialog({ name, native, value, language, onApply, onClose }: {
  name: string; native: NativeReasoning; value: Selection['reasoning']; language: 'zh' | 'en';
  onApply(value: Selection['reasoning']): void; onClose(): void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [draft, setDraft] = useState(value);
  const [budget, setBudget] = useState(value?.kind === 'budget' ? String(value.tokens) : '');
  const [error, setError] = useState('');
  const capability = native.kind === 'discrete'
    ? text('可选思考档位', 'Selectable reasoning levels')
    : native.kind === 'toggle'
      ? text('可开启或关闭思考', 'Reasoning can be turned on or off')
      : native.kind === 'budget'
        ? text('可设置思考预算', 'Reasoning budget can be set')
        : text('思考设置由模型固定', 'Reasoning is fixed by the model');
  function apply() {
    if (native.kind === 'budget') {
      const tokens = Number(budget);
      if (!budget.trim() || !Number.isSafeInteger(tokens) || tokens < native.minimum_tokens || tokens > native.maximum_tokens || (tokens - native.minimum_tokens) % native.step_tokens !== 0) {
        setError(text(`请输入 ${native.minimum_tokens}–${native.maximum_tokens} 之间、步长为 ${native.step_tokens} 的预算。`, `Enter a budget from ${native.minimum_tokens} to ${native.maximum_tokens}, in steps of ${native.step_tokens}.`));
        return;
      }
      onApply({ kind: 'budget', tokens });
    } else if (native.kind === 'fixed') onClose();
    else if (!draft || (native.kind === 'discrete' && (draft.kind !== 'profile' || !native.profiles.includes(draft.profile))) || (native.kind === 'toggle' && draft.kind !== 'toggle')) setError(text('请选择思考设置。', 'Choose a reasoning setting.'));
    else onApply(draft);
  }
  return <Dialog open title={`${text('思考强度', 'Reasoning')} · ${name}`} description={capability} closeLabel={text('关闭思考设置', 'Close reasoning settings')} onClose={onClose}
    footer={<><button type="button" className="btn" onClick={onClose}>{text('取消', 'Cancel')}</button><button type="submit" form="route-reasoning" className="btn btn-primary">{text('应用', 'Apply')}</button></>}>
    <form noValidate id="route-reasoning" onSubmit={event => { event.preventDefault(); apply(); }}>
      {native.kind === 'fixed' && <div className="callout"><UiIcon name="info" /><span>{text('这个模型没有可调的思考参数。', 'This model has no adjustable reasoning control.')}</span></div>}
      {native.kind === 'discrete' && <div className="segmented reasoning-options">{native.profiles.map(profile => <button className={`segment${draft?.kind === 'profile' && draft.profile === profile ? ' active' : ''}`} type="button" key={profile} data-reasoning-profile={profile} aria-pressed={draft?.kind === 'profile' && draft.profile === profile} onClick={() => { setDraft({ kind: 'profile', profile }); setError(''); }}>{profile}</button>)}</div>}
      {native.kind === 'toggle' && <div className="segmented reasoning-options">{[false, true].map(enabled => <button className={`segment${draft?.kind === 'toggle' && draft.enabled === enabled ? ' active' : ''}`} type="button" key={String(enabled)} aria-pressed={draft?.kind === 'toggle' && draft.enabled === enabled} onClick={() => { setDraft({ kind: 'toggle', enabled }); setError(''); }}>{enabled ? text('开启', 'On') : text('关闭', 'Off')}</button>)}</div>}
      {native.kind === 'budget' && <label className="field"><span className="field-label">{text('思考预算', 'Reasoning budget')}</span><div className="inline-input"><input className="input" data-autofocus type="number" min={native.minimum_tokens} max={native.maximum_tokens} step={native.step_tokens} value={budget} onChange={event => { setBudget(event.target.value); setError(''); }} /><span className="muted">tokens</span></div><span className="field-help">{text(`范围 ${native.minimum_tokens}–${native.maximum_tokens}，步长 ${native.step_tokens}`, `Range ${native.minimum_tokens}–${native.maximum_tokens}, step ${native.step_tokens}`)}</span></label>}
      {error && <p role="alert" className="oc-inline-error">{error}</p>}
    </form>
  </Dialog>;
}
