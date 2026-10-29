import { useEffect, useState } from 'react';
import { assessmentStandard, resolveAssessment, stockAssessment } from './native-protocol';
import { Disclosure } from '../../ui';
import { defaultJudgment, type Judgment } from './types';

export function judgmentSummary(value: Judgment, dual: boolean, language: 'zh' | 'en'): string {
  return language === 'zh'
    ? `${dual ? `简单概率 ≥ ${value.degree.simple_threshold_millis / 1000} · ` : ''}胜任度 ≥ ${value.competence.floor_millis / 1000}`
    : `${dual ? `P(simple) ≥ ${value.degree.simple_threshold_millis / 1000} · ` : ''}Competence ≥ ${value.competence.floor_millis / 1000}`;
}
// An empty numeric edit is temporary UI state, never a NaN/null in saved editor data.
function ProbabilityInput({ value, field, onChange, language }: { value: number; field: string; onChange(value: number): void; language: 'zh' | 'en' }) {
  const [text, setText] = useState(String(value / 1000));
  useEffect(() => setText(String(value / 1000)), [value]);
  const invalid = text === '' || !Number.isFinite(Number(text)) || Number(text) < 0 || Number(text) > 1;
  return <><input aria-invalid={invalid} className="input" type="number" min={0} max={1} step={0.01} value={text} data-decision-field={field}
    onChange={event => { const raw = event.target.value; setText(raw); const number = Number(raw); if (raw !== '' && Number.isFinite(number) && number >= 0 && number <= 1) onChange(Math.round(number * 1000)); }}
    onBlur={() => setText(String(value / 1000))} />{invalid && <span className="oc-inline-error" role="alert">{language === 'zh' ? '请输入 0 到 1；离开此字段时恢复上一个有效值。' : 'Enter 0 to 1; leaving this field restores the last valid value.'}</span>}</>;
}

export function JudgmentFields({ value, onChange, id, dual = true, disabled = false, hideThreshold = false, language, native = false }: {
  value: Judgment; onChange(value: Judgment): void; id: string; dual?: boolean; disabled?: boolean; hideThreshold?: boolean; language: 'zh' | 'en'; native?: boolean;
}) {
  value = resolveAssessment(value, native);
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const prompt = (key: string, label: string, content: string, change: (text: string) => void) => <label className="field" key={key}><span className="field-label">{label}</span><textarea className="input" rows={3} value={content} data-decision-field={`${id}-${key}`} onChange={e => change(e.target.value)} /></label>;
  return <fieldset className="decision-judgment-fields" disabled={disabled}>
    <div className="decision-thresholds">
      {dual && !hideThreshold && <label className="field"><span className="field-label">{t('简单概率阈值', 'Simple probability threshold')}</span><ProbabilityInput language={language} value={value.degree.simple_threshold_millis} field={`${id}-simple-threshold`} onChange={simple_threshold_millis => onChange({ ...value, degree: { ...value.degree, simple_threshold_millis } })} /><span className="field-help">{t('达到此值，且没有适用的低胜任评分时使用常规模型。', 'Use regular models when this threshold is met and no applicable low score is present.')}</span></label>}
      <label className="field"><span className="field-label">{t('胜任下限', 'Competence floor')}</span><ProbabilityInput language={language} value={value.competence.floor_millis} field={`${id}-floor`} onChange={floor_millis => onChange({ ...value, competence: { ...value.competence, floor_millis } })} /><span className="field-help">{dual ? t('完整评分低于此值记为低分；是否保护由当前判断规则决定。', 'Complete scores below this value count as low. The current decision rules determine whether protection applies.') : t('用于观察该分支的模型表现；添加主力模型后可用于保护。', 'Used to observe model performance. Add primary models to enable protection.')}</span></label>
    </div>
    {dual && <Disclosure label={t('什么是简单、复杂任务', 'What counts as simple or complex')} language={language}>
      {prompt('degree-instructions', t('判断要求', 'Decision instructions'), value.degree.instructions, instructions => onChange({ ...value, degree: { ...value.degree, instructions } }))}
      {prompt('simple', t('简单任务', 'Simple tasks'), value.degree.simple, simple => onChange({ ...value, degree: { ...value.degree, simple } }))}
      {prompt('complex', t('复杂任务', 'Complex tasks'), value.degree.complex, complex => onChange({ ...value, degree: { ...value.degree, complex } }))}
      <button className="btn" type="button" onClick={() => onChange({ ...value, degree: { ...structuredClone(defaultJudgment.degree), simple_threshold_millis: value.degree.simple_threshold_millis } })}>{t('恢复默认', 'Reset to default')}</button>
    </Disclosure>}
    <Disclosure label={t('如何判断胜任', 'How competence is assessed')} language={language}>
      <p className="field-help">{assessmentStandard(value) === 'default' ? t('默认评分', 'Default assessment') : t('自定义评分', 'Custom assessment')}</p>
      {prompt('competence-instructions', t('评分要求', 'Assessment instructions'), value.competence.instructions, instructions => onChange({ ...value, competence: { ...value.competence, standard: 'custom', instructions } }))}
      {value.competence.criteria.map((criterion, index) => prompt(`criterion-${index}`, [t('不胜任 · 0', 'Not competent · 0'), t('部分胜任 · 0.5', 'Partly competent · 0.5'), t('胜任 · 1', 'Competent · 1')][index], criterion, text => onChange({ ...value, competence: { ...value.competence, standard: 'custom', criteria: value.competence.criteria.map((c, i) => i === index ? text : c) as Judgment['competence']['criteria'] } })))}
      <button className="btn" type="button" onClick={() => onChange(stockAssessment(value, native))}>{t('恢复默认', 'Reset to default')}</button>
    </Disclosure>
  </fieldset>;
}
