import { Disclosure } from '../../ui';
import { defaultJudgment, type Judgment } from './types';

export function judgmentSummary(value: Judgment, dual: boolean, language: 'zh' | 'en'): string {
  return language === 'zh'
    ? `${dual ? `简单概率 ≥ ${value.degree.simple_threshold_millis / 1000} · ` : ''}胜任度 ≥ ${value.competence.floor_millis / 1000}`
    : `${dual ? `P(simple) ≥ ${value.degree.simple_threshold_millis / 1000} · ` : ''}Competence ≥ ${value.competence.floor_millis / 1000}`;
}
export function JudgmentFields({ value, onChange, id, dual = true, disabled = false, language }: {
  value: Judgment; onChange(value: Judgment): void; id: string; dual?: boolean; disabled?: boolean; language: 'zh' | 'en';
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const prompt = (key: string, label: string, content: string, change: (text: string) => void) => <label className="field" key={key}><span className="field-label">{label}</span><textarea className="input" rows={3} value={content} data-decision-field={`${id}-${key}`} onChange={e => change(e.target.value)} /></label>;
  return <fieldset className="decision-judgment-fields" disabled={disabled}>
    <div className="decision-thresholds">
      {dual && <label className="field"><span className="field-label">{t('简单概率阈值', 'Simple probability threshold')}</span><input className="input" type="number" min={0} max={1} step={0.01} value={Number.isFinite(value.degree.simple_threshold_millis) ? value.degree.simple_threshold_millis / 1000 : ''} data-decision-field={`${id}-simple-threshold`} onChange={e => onChange({ ...value, degree: { ...value.degree, simple_threshold_millis: e.target.value === '' ? NaN : Math.round(Number(e.target.value) * 1000) } })} /><span className="field-help">{t('达到此值，且没有适用的低胜任评分时使用常规（省钱）模型。', 'Use regular (economy) models when this threshold is met and no applicable low score is present.')}</span></label>}
      <label className="field"><span className="field-label">{t('胜任度下限', 'Competence floor')}</span><input className="input" type="number" min={0} max={1} step={0.01} value={Number.isFinite(value.competence.floor_millis) ? value.competence.floor_millis / 1000 : ''} data-decision-field={`${id}-floor`} onChange={e => onChange({ ...value, competence: { ...value.competence, floor_millis: e.target.value === '' ? NaN : Math.round(Number(e.target.value) * 1000) } })} /><span className="field-help">{dual ? t('上一阶段的完整评分低于此值，本轮使用主力模型。', 'Use primary models for this turn when the applicable previous stage score falls below this floor.') : t('用于观察该分支的模型表现；添加主力模型后可用于保护。', 'Used to observe model performance. Add primary models to enable protection.')}</span></label>
    </div>
    {dual && <Disclosure label={t('什么是简单、复杂任务', 'What counts as simple or complex')} language={language}>
      {prompt('degree-instructions', t('判断要求', 'Decision instructions'), value.degree.instructions, instructions => onChange({ ...value, degree: { ...value.degree, instructions } }))}
      {prompt('simple', t('简单任务', 'Simple tasks'), value.degree.simple, simple => onChange({ ...value, degree: { ...value.degree, simple } }))}
      {prompt('complex', t('复杂任务', 'Complex tasks'), value.degree.complex, complex => onChange({ ...value, degree: { ...value.degree, complex } }))}
      <button className="btn" type="button" onClick={() => onChange({ ...value, degree: { ...structuredClone(defaultJudgment.degree), simple_threshold_millis: value.degree.simple_threshold_millis } })}>{t('恢复默认提示词', 'Restore default prompts')}</button>
    </Disclosure>}
    <Disclosure label={t('如何判断胜任', 'How competence is assessed')} language={language}>
      {prompt('competence-instructions', t('评分要求', 'Assessment instructions'), value.competence.instructions, instructions => onChange({ ...value, competence: { ...value.competence, instructions } }))}
      {value.competence.criteria.map((criterion, index) => prompt(`criterion-${index}`, [t('不胜任 · 0', 'Not competent · 0'), t('部分胜任 · 0.5', 'Partly competent · 0.5'), t('胜任 · 1', 'Competent · 1')][index], criterion, text => onChange({ ...value, competence: { ...value.competence, criteria: value.competence.criteria.map((c, i) => i === index ? text : c) as Judgment['competence']['criteria'] } })))}
      <button className="btn" type="button" onClick={() => onChange({ ...value, competence: { ...structuredClone(defaultJudgment.competence), floor_millis: value.competence.floor_millis } })}>{t('恢复默认提示词', 'Restore default prompts')}</button>
    </Disclosure>
  </fieldset>;
}
