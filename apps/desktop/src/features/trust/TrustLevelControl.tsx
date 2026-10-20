import { Disclosure } from '../../ui';
import { TrustReplayPreview, type ReplayScope } from './TrustReplayPreview';
import { trustLevels, trustLevelLabel } from './levels';
import type { Classifier, Judgment } from '../decision-services/types';
import { protectionExplanation } from './protection-copy';
import './trust.css';

export function TrustLevelControl({ value, classifier, onChange, language, id, complete, evidenceError, replayScope }: {
  classifier: Classifier;
  value: Judgment; onChange(value: Judgment): void; language: 'zh' | 'en'; id: string;
  complete?: number; evidenceError?: boolean; replayScope?: ReplayScope;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const threshold = value.degree.simple_threshold_millis;
  const change = (simple_threshold_millis: number) => onChange({ ...value, degree: { ...value.degree, simple_threshold_millis } });
  const index = trustLevels.findIndex(level => level.threshold_millis === threshold);
  // A custom value keeps the nearest track position for orientation; only an explicit move rewrites it.
  const nearest = trustLevels.reduce((best, level, i) => Math.abs(level.threshold_millis - threshold) < Math.abs(trustLevels[best].threshold_millis - threshold) ? i : best, 0);
  const floor = value.competence.floor_millis / 1000;
  const evidence = complete === undefined ? (evidenceError ? t('暂时无法读取当前版本的证据。', 'Evidence for the active version is temporarily unavailable.') : t('当前版本的证据尚未载入。', 'Evidence for the active version has not loaded yet.'))
    : complete === 0 ? t('还没有记录的证据支持此档位。', 'No recorded evidence supports this level yet.')
    : t(`当前版本已有 ${complete} 个评分完整的常规阶段；档位由你决定，依据见「模型表现」。`, `The active version has ${complete} fully scored Regular stages. The level is your choice; see Model performance for the evidence.`);
  return <section className="trust-control" data-trust-control={id}>
    <div className="trust-control-head"><h3>{t('信任度 · 交给常规模型的程度', 'Trust · how much goes to Regular')}</h3><span className={`badge no-dot ${index < 0 ? 'warn' : 'info'}`}>{trustLevelLabel(threshold, language)}</span></div>
    <p className="trust-control-help">{t('越往右，越多任务交给常规模型。实际保存的是简单概率阈值：任务越简单，越先交给它。只改变路由，不影响 Agent 权限、工具审批和沙箱。', 'Further right sends more tasks to Regular. The saved value is a simple-probability threshold: simpler tasks go first. Only routing changes; agent permissions, tool approval and sandboxing do not.')}</p>
    <input className={`trust-slider${index < 0 ? ' custom' : ''}`} data-step={index < 0 ? nearest : index} type="range" min={0} max={trustLevels.length - 1} step={1} value={index < 0 ? nearest : index} aria-label={t('信任度', 'Trust level')} aria-valuetext={trustLevelLabel(threshold, language)} onChange={event => change(trustLevels[Number(event.target.value)].threshold_millis)} />
    <div className="trust-levels" role="group" aria-label={t('信任档位', 'Trust level')}>
      {trustLevels.map(level => <button key={level.id} type="button" aria-pressed={threshold === level.threshold_millis} onClick={() => change(level.threshold_millis)}><strong>{level.label[language]}</strong><span>≥ {(level.threshold_millis / 1000).toFixed(2)}</span></button>)}
    </div>
    <Disclosure className="trust-exact" label={t('精确值', 'Exact value')} language={language}><div className="trust-exact-body">
      <label><span>{t('简单概率阈值', 'Simple probability threshold')}</span><input className="input" type="number" min={0} max={1} step={0.001} value={Number.isFinite(threshold) ? threshold / 1000 : ''} data-decision-field={`${id}-simple-threshold`} onChange={event => change(event.target.value === '' ? NaN : Math.round(Number(event.target.value) * 1000))} /></label>
      <span>{t('不在档位上的值显示为「自定义」，保存时不会被改写。', 'Values between levels show as Custom and are saved as entered.')}</span>
    </div></Disclosure>
    <div className="trust-safety-net">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6z" /></svg>
      <div><strong>{t(`安全网 · 胜任度下限 ${floor}`, `Safety net · competence floor ${floor}`)}</strong><span>{t('不随信任度变化。', 'Unchanged by trust. ')}{protectionExplanation(classifier, language)}</span></div>
    </div>
    {replayScope && <TrustReplayPreview scope={replayScope} threshold={threshold} language={language} />}
    <p className="trust-control-evidence" role="status">{evidence}</p>
  </section>;
}
