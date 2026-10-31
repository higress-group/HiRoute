import { useRef } from 'react';
import { reasoningValue } from '../model-connections/form-support';
import type { NativeReasoning } from '../model-connections/types';

export function ModelCapabilityInput({ field, label, value, reference, language, onChange }: {
  field: string; label: string; value: unknown; reference: unknown; language: 'zh' | 'en'; onChange(value: unknown): void;
}) {
  type Native = Exclude<NativeReasoning, {kind: 'unknown'}>;
  const drafts = useRef<Partial<Record<Native['kind'], Native>>>({});
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (['tool', 'vision', 'streaming'].includes(field)) return <select className="select" aria-label={label + ' · ' + t('自定义', 'Custom')} value={value === true ? 'true' : value === false ? 'false' : 'unknown'} onChange={e => onChange(e.target.value === 'unknown' ? null : e.target.value === 'true')}>
    <option value="true">{t('支持', 'Supported')}</option><option value="false">{t('不支持', 'Not supported')}</option><option value="unknown">{t('未知', 'Unknown')}</option>
  </select>;
  if (field !== 'native_reasoning') return <input className="input" aria-label={label + ' · ' + t('自定义上限', 'Custom limit')} type="number" min="1" step="1" placeholder={t('未知', 'Unknown')} value={typeof value === 'number' ? value : ''} onChange={e => onChange(e.target.value === '' ? null : Number(e.target.value))} />;
  const native = value as Native | null;
  const original = reference as Native | null;
  return <div className="model-default-reasoning">
    <select className="select" aria-label={t('自定义思考方式', 'Custom reasoning kind')} value={native?.kind ?? 'unknown'} onChange={e => {
      if (native) drafts.current[native.kind] = native;
      const kind = e.target.value as NativeReasoning['kind'];
      const previous = kind === 'unknown' ? undefined : drafts.current[kind] ?? (original?.kind === kind ? original : undefined);
      onChange(reasoningValue(kind, previous));
    }}>
      <option value="unknown">{t('未知', 'Unknown')}</option><option value="fixed">{t('供应商固定', 'Provider fixed')}</option><option value="discrete">{t('可选档位', 'Discrete levels')}</option><option value="toggle">{t('开关', 'On / off')}</option><option value="budget">{t('Token 预算', 'Token budget')}</option>
    </select>
    {native?.kind === 'discrete' && <>
      <label className="field"><span className="field-label">{t('支持的档位 · 从低到高', 'Supported levels · low to high')}</span><input className="input" data-capability-profiles defaultValue={native.profiles.join(', ')} placeholder="low, medium, high" onChange={e => { const profiles = e.target.value.split(',').map(s => s.trim()).filter(Boolean); onChange({...native, profiles, default_profile: native.default_profile && profiles.includes(native.default_profile) ? native.default_profile : undefined}); }} /><span className="field-help">{t('用逗号分隔，保留原生档位名称。', 'Comma separated; use native level names.')}</span></label>
      <label className="field"><span className="field-label">{t('模型默认档位（可选）', 'Model default level (optional)')}</span><select className="select" value={native.default_profile ?? ''} onChange={e => onChange({...native, default_profile: e.target.value || undefined})}><option value="">{t('未指定', 'Not specified')}</option>{native.profiles.map((p,i) => <option key={`${p}-${i}`} value={p}>{p}</option>)}</select></label>
    </>}
    {native?.kind === 'budget' && (['minimum_tokens', 'maximum_tokens', 'step_tokens'] as const).map((key,i) => <label className="field" key={key}><span className="field-label">{t(['最小预算', '最大预算', '预算步长'][i], ['Minimum budget', 'Maximum budget', 'Budget step'][i])}</span><input className="input" type="number" min="1" step="1" defaultValue={native[key]} onChange={e => onChange({...native, [key]: Number(e.target.value)})} /></label>)}
    {native?.kind === 'fixed' && <span className="field-help">{t('不发送可调整的思考参数。', 'No adjustable reasoning parameter is sent.')}</span>}
  </div>;
}
