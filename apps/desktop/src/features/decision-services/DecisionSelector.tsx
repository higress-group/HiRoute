import { useId, useRef } from 'react';
import { UiIcon } from '../../ui';
import { DecisionIcon } from './DecisionIcon';
import { classifierFor, emptyService, type Classifier } from './types';
import type { DecisionServiceView } from './contracts';
import { isNativeRecord, recordProvider, serviceChoiceLabels } from './form-state';
import { providerLabel, type DecisionKind, type OpenDecisionConnection } from './presentation';

export function DecisionSelector({ classifier, smart, services, language, onChange, onOpenServices, error, announceError = true, trialFocus = false, allowHeuristic = true }: {
  classifier: Classifier; smart: boolean; allowHeuristic?: boolean; services: DecisionServiceView[]; language: 'zh' | 'en';
  onChange(classifier: Classifier): void; onOpenServices?: OpenDecisionConnection; error?: string; announceError?: boolean; trialFocus?: boolean;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const errorId = useId();
  const selected = classifier.kind === 'local_rules' ? null : classifier.service;
  const mode: DecisionKind | 'local_rules' = classifier.kind === 'local_rules' ? 'local_rules' : isNativeRecord(classifier.service) ? 'system_one' : 'custom';
  const previous = useRef<Partial<Record<DecisionKind, Classifier>>>({});
  const matches = (view: DecisionServiceView, kind: DecisionKind | 'local_rules') => kind === 'custom' ? !isNativeRecord(view.service) : kind === 'system_one' && isNativeRecord(view.service);
  const choices = services.filter(view => matches(view, mode));
  const labels = serviceChoiceLabels(choices, language);
  const latest = selected && choices.find(view => view.service.id === selected.id)?.service;
  const select = (service: DecisionServiceView['service']) => onChange(classifierFor(structuredClone(service)));
  function changeMode(next: DecisionKind | 'local_rules') {
    if (next === mode) return;
    if (mode !== 'local_rules') previous.current[mode] = structuredClone(classifier);
    if (next === 'local_rules') { onChange({ kind: 'local_rules' }); return; }
    const first = services.find(view => matches(view, next))?.service;
    const empty = emptyService();
    if (next === 'custom') empty.connection = { kind: 'custom', endpoint: '', timeout_ms: 10000 };
    onChange(previous.current[next] ?? classifierFor(structuredClone(first ?? empty)));
  }
  return <section className="editor-section" data-route-group="classifier">
    <h3>{t('判断方式', 'Decision method')}</h3>
    <div className="decision-methods" role="group" aria-label={t('判断方式', 'Decision method')}>
      <button className="btn" type="button" aria-pressed={mode === 'system_one'} onClick={() => changeMode('system_one')}>{t('决策模型', 'Decision model')}</button>
      {smart && allowHeuristic && <button className="btn" type="button" aria-pressed={mode === 'local_rules'} onClick={() => changeMode('local_rules')}>{t('启发式规则', 'Heuristic rules')}</button>}
      <button className="btn" type="button" aria-pressed={mode === 'custom'} onClick={() => changeMode('custom')}>{t('自定义扩展', 'Custom extension')}</button>
    </div>
    <p className="field-help">{mode === 'local_rules' ? t('按任务特征判断简单或复杂，不产生模型胜任评分。', 'Classifies simple or complex tasks from structural signals; no competence scoring.') : mode === 'custom' ? t('扩展直接选择任务类别和常规或主力组。可选评分仅用于观察，HiRoute 在选定组中选择并执行模型。', 'Your extension chooses the category and regular or primary group. Optional scores are observation only; HiRoute selects and executes a model within that group.') : (smart ? t('判断任务简单或复杂，并评估上一阶段的胜任情况。', 'Judge task difficulty and assess competence in the preceding stage.') : t('先按任务条件选择分支，再判断该分支需要常规还是主力模型。', 'Choose a task branch, then decide whether regular or primary models are needed.'))}</p>
    {mode !== 'local_rules' && <>
      <div className="decision-selector"><DecisionIcon provider={selected ? recordProvider(selected) : undefined} language={language} /><label className="field"><span className="field-label">{mode === 'custom' ? t('使用的扩展', 'Extension') : t('使用的决策模型', 'Decision model')}</span><select className="input" data-trial-focus={trialFocus || undefined} aria-invalid={!!error} aria-describedby={error ? errorId : undefined} value={selected?.name ? selected.id : ''} onChange={event => { const view = choices.find(item => item.service.id === event.target.value); if (view) select(view.service); }}><option value="">{t('请选择已保存的连接', 'Select a saved connection')}</option>{selected?.name && !choices.some(item => item.service.id === selected.id) && <option value={selected.id}>{selected.name}</option>}{choices.map(item => <option value={item.service.id} key={item.service.id}>{labels.get(item.service.id)}</option>)}</select></label></div>
      {selected?.name && <div className="decision-selection-meta"><span className="field-help">{providerLabel(selected.connection, language)} · {t('发布时固定所选配置', 'The selected configuration is pinned on publication')}</span>{latest && latest.revision > selected.revision && <button type="button" className="btn btn-quiet" onClick={() => select(latest)}>{t('使用最新配置', 'Use latest configuration')}</button>}</div>}
      {onOpenServices && <button className="btn btn-quiet" type="button" onClick={() => onOpenServices(mode, service => select(service))}><UiIcon name="plus" />{mode === 'custom' ? t('接入自定义扩展', 'Connect custom extension') : t('添加决策模型', 'Add decision model')}</button>}
    </>}
    {error && <p id={errorId} className="oc-inline-error" role={announceError ? 'alert' : undefined}>{error}</p>}
  </section>;
}
