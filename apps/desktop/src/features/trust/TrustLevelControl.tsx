import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { Disclosure } from '../../ui';
import { TrustReplayPreview, type ReplayScope } from './TrustReplayPreview';
import { trustLevels } from './levels';
import type { Classifier, Judgment } from '../decision-services/types';
import { protectionExplanation } from './protection-copy';
import './trust.css';

const manualPresets = [...trustLevels, { id: 'entrusted', threshold_millis: 0, label: { zh: '托付', en: 'Entrusted' } }];

function ExactThreshold({ threshold, change, language, id }: { threshold: number; change(value: number): void; language: 'zh' | 'en'; id: string }) {
  const [input, setInput] = useState(String(threshold / 1000));
  const editing = useRef(false);
  useEffect(() => { if (!editing.current) setInput(String(threshold / 1000)); }, [threshold]);
  const number = Number(input);
  const millis = Math.round(number * 1000);
  const valid = input !== '' && Number.isFinite(number) && number >= 0 && number <= 1 && Math.abs(number * 1000 - millis) < 1e-7;
  return <div className="trust-exact-body">
    <label><span>{language === 'zh' ? '简单概率阈值' : 'Simple probability threshold'}</span><input className="input" type="number" min={0} max={1} step={0.001} value={input} aria-invalid={!valid} data-decision-field={`${id}-simple-threshold`} onFocus={() => { editing.current = true; }} onChange={event => {
      const raw = event.target.value;
      setInput(raw);
      const next = Number(raw), rounded = Math.round(next * 1000);
      if (raw !== '' && Number.isFinite(next) && next >= 0 && next <= 1 && Math.abs(next * 1000 - rounded) < 1e-7) change(rounded);
    }} onBlur={() => { editing.current = false; setInput(String(threshold / 1000)); }} /></label>
    {!valid && <span className="oc-inline-error" role="alert">{language === 'zh' ? '请输入 0 到 1，最多三位小数。' : 'Enter a value from 0 to 1 with at most three decimal places.'}</span>}
  </div>;
}

export function TrustLevelControl({ value, classifier, onChange, language, id, complete, evidenceError, replayScope }: {
  classifier: Classifier;
  value: Judgment; onChange(value: Judgment): void; language: 'zh' | 'en'; id: string;
  complete?: number; evidenceError?: boolean; replayScope?: ReplayScope;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const threshold = value.degree.simple_threshold_millis;
  const change = (simple_threshold_millis: number) => onChange({ ...value, degree: { ...value.degree, simple_threshold_millis } });
  const floor = value.competence.floor_millis / 1000;
  const evidence = complete === undefined ? (evidenceError ? t('暂时无法读取当前版本的证据。', 'Evidence for the active version is temporarily unavailable.') : t('当前版本的证据尚未载入。', 'Evidence for the active version has not loaded yet.'))
    : complete === 0 ? t('当前版本还没有记录的证据。', 'No recorded evidence is available for the active version yet.')
    : t(`当前版本已有 ${complete} 个评分完整的常规阶段；信任设置由你决定，依据见「模型表现」。`, `The active version has ${complete} fully scored Regular stages. The setting is your choice; see Model performance for the evidence.`);
  return <section className="trust-control" data-trust-control={id}>
    <div className="trust-control-head"><h3>{t('信任度 · 交给常规模型的程度', 'Trust · how much goes to Regular')}</h3></div>
    <p className="trust-control-help">{t('越往右，越多任务交给常规模型。实际保存的是简单概率阈值：任务越简单，越先交给它。只改变路由，不影响 Agent 权限、工具审批和沙箱。', 'Further right sends more tasks to Regular. The saved value is a simple-probability threshold: simpler tasks go first. Only routing changes; agent permissions, tool approval and sandboxing do not.')}</p>
    <div className="trust-endpoints"><span>{t('谨慎', 'Cautious')}</span><span>{t('托付', 'Entrusted')}</span></div>
    <input className="trust-slider" style={{ '--fill': `${(1000 - threshold) / 10}%` } as CSSProperties} type="range" min={0} max={1000} step={1} value={1000 - threshold} aria-label={t('信任度', 'Trust')} aria-valuetext={t(`简单概率阈值 ${threshold / 1000}；向右增加信任，降低阈值`, `Simple probability threshold ${threshold / 1000}; moving right increases trust and lowers the threshold`)} onChange={event => change(1000 - Number(event.target.value))} />
    <p className="trust-threshold">{t('简单概率阈值', 'Simple probability threshold')}: {threshold / 1000}</p>
    <div className="trust-levels" role="group" aria-label={t('信任档位', 'Trust level')}>
      {manualPresets.map(level => <button key={level.id} type="button" aria-pressed={threshold === level.threshold_millis} onClick={() => change(level.threshold_millis)}><strong>{level.label[language]}</strong></button>)}
    </div>
    <Disclosure className="trust-exact" label={t('精确值', 'Exact value')} language={language}><ExactThreshold threshold={threshold} change={change} language={language} id={id} /></Disclosure>
    <div className="trust-safety-net">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6z" /></svg>
      <div><strong>{t(`低分保护 · 胜任下限 ${floor}`, `Low-score protection · competence floor ${floor}`)}</strong><span>{t('即使阈值为零，适用的低分保护仍然生效。', 'Applicable low-score protection still applies at zero. ')}{protectionExplanation(classifier, language)}</span></div>
    </div>
    {replayScope && <TrustReplayPreview scope={replayScope} threshold={threshold} language={language} />}
    <p className="trust-control-evidence" role="status">{evidence}</p>
  </section>;
}
