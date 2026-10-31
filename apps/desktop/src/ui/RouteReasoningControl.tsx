import { useId } from 'react';
import type { Selection } from '../plan-editor';
import type { NativeReasoning } from './ReasoningDialog';
import { UiIcon } from './UiIcon';

/** Route choices update the editor draft; only the route's Save/Publish persists them. */
export function RouteReasoningControl({ name, native, value, followRequest, loading, language, onChange, onEditBudget }: {
  name: string;
  native?: NativeReasoning;
  value: Selection['reasoning'];
  followRequest: boolean;
  loading: boolean;
  language: 'zh' | 'en';
  onChange(value: Selection['reasoning']): void;
  onEditBudget(): void;
}) {
  const id = useId();
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (!native || native.kind === 'fixed') return <div className="route-reasoning-field route-reasoning-readonly">
    <span className="route-reasoning-label">{text('思考', 'Reasoning')}</span>
    <span className="route-reasoning-value">{native ? text('供应商默认', 'Provider default') : loading ? text('正在读取…', 'Loading…') : text('暂不可用', 'Unavailable')}</span>
  </div>;

  const subject = native.kind === 'budget' ? text('思考预算', 'Reasoning budget')
    : native.kind === 'toggle' ? text('思考开关', 'Reasoning switch') : text('思考档位', 'Reasoning level');
  const label = subject;
  const accessibleLabel = `${name} · ${subject} · ${followRequest ? text('默认', 'Default') : text('固定', 'Fixed')}`;
  const selected = native.kind === 'discrete' && value?.kind === 'profile' ? value.profile
    : native.kind === 'toggle' && value?.kind === 'toggle' ? (value.enabled ? 'enabled' : 'disabled') : '';
  const mismatched = !!value && (native.kind === 'discrete'
    ? value.kind !== 'profile' || !native.profiles.includes(value.profile)
    : native.kind === 'toggle' && value.kind !== 'toggle');

  return <div className="route-reasoning-field">
    <label className="route-reasoning-label" htmlFor={id}>{label}</label>
    {native.kind === 'budget' ? <button id={id} type="button" className="effort-select route-reasoning-budget" aria-label={accessibleLabel} onClick={onEditBudget}>
      <span>{value?.kind === 'budget' ? `${value.tokens} tokens` : text('设置预算', 'Set budget')}</span>
      {value?.kind === 'budget' && <span className="route-reasoning-edit">{text('编辑', 'Edit')}</span>}
    </button> : <span className="route-reasoning-picker">
      <select id={id} className="effort-select" aria-label={accessibleLabel} aria-invalid={mismatched || undefined} required value={selected}
        disabled={native.kind === 'discrete' && native.profiles.length === 0} title={selected || undefined}
        onChange={event => {
          const chosen = event.target.value;
          if (native.kind === 'discrete' && native.profiles.includes(chosen)) onChange({ kind: 'profile', profile: chosen });
          else if (native.kind === 'toggle' && (chosen === 'enabled' || chosen === 'disabled')) onChange({ kind: 'toggle', enabled: chosen === 'enabled' });
        }}>
        {!selected && <option value="" disabled>{mismatched ? text('已保存设置不匹配', 'Saved setting unsupported') : text('请选择', 'Choose a setting')}</option>}
        {native.kind === 'discrete' && selected && !native.profiles.includes(selected) && <option value={selected} disabled>{selected} · {text('当前不支持', 'Unsupported')}</option>}
        {native.kind === 'discrete' ? native.profiles.map(profile => <option key={profile} value={profile}>{profile}</option>)
          : <><option value="disabled">{text('关闭', 'Off')}</option><option value="enabled">{text('开启', 'On')}</option></>}
      </select>
      <UiIcon name="chevron" />
    </span>}
  </div>;
}
