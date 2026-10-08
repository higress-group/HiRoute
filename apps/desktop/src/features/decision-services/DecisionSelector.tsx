import { useRef } from 'react';
import { UiIcon } from '../../ui';
import { DecisionIcon } from './DecisionIcon';
import { emptyService, type Classifier, type DecisionService } from './types';
import { providerLabel, type DecisionKind, type OpenDecisionConnection } from './presentation';

export function DecisionSelector({ classifier, smart, services, language, onChange, onOpenServices, error }: {
  classifier: Classifier; smart: boolean; services: DecisionService[]; language: 'zh' | 'en';
  onChange(classifier: Classifier): void; onOpenServices?: OpenDecisionConnection; error?: string;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const selected = classifier.kind === 'decision_service' ? classifier.service : null;
  const mode = classifier.kind === 'local_rules' ? 'local_rules' : classifier.service.connection.kind;
  const previous = useRef<Partial<Record<DecisionKind, Classifier>>>({});
  const choices = services.filter(service => service.connection.kind === mode);
  const latest = selected && choices.find(service => service.id === selected.id);
  function changeMode(next: DecisionKind | 'local_rules') {
    if (next === mode) return;
    if (mode !== 'local_rules') previous.current[mode] = structuredClone(classifier);
    if (next === 'local_rules') { onChange({ kind: 'local_rules' }); return; }
    const first = services.find(service => service.connection.kind === next);
    const empty = emptyService();
    if (next === 'custom') empty.connection = { kind: 'custom', endpoint: '', timeout_ms: 10000 };
    onChange(previous.current[next] ?? { kind: 'decision_service', service: structuredClone(first ?? empty) });
  }
  return <section className="editor-section" data-route-group="classifier">
    <h3>{t('判断方式', 'Decision method')}</h3>
    <div className="decision-methods" role="group" aria-label={t('判断方式', 'Decision method')}>
      <button className="btn" type="button" aria-pressed={mode === 'system_one'} onClick={() => changeMode('system_one')}>{t('决策模型', 'Decision model')}</button>
      {smart && <button className="btn" type="button" aria-pressed={mode === 'local_rules'} onClick={() => changeMode('local_rules')}>{t('启发式规则', 'Heuristic rules')}</button>}
      <button className="btn" type="button" aria-pressed={mode === 'custom'} onClick={() => changeMode('custom')}>{t('自定义扩展', 'Custom extension')}</button>
    </div>
    <p className="field-help">{mode === 'local_rules' ? t('按任务特征判断简单或复杂，不产生模型胜任评分。', 'Classifies simple or complex tasks from structural signals; no competence scoring.') : mode === 'custom' ? t('由扩展按传入的任务条件和判断标准进行决策，HiRoute 负责选择并执行模型。', 'Your extension follows the supplied task conditions and judgment standards. HiRoute selects and executes the models.') : (smart ? t('判断任务简单或复杂，并评估上一阶段的胜任情况。', 'Judge task difficulty and assess competence in the preceding stage.') : t('先按任务条件选择分支，再判断该分支需要常规还是主力模型。', 'Choose a task branch, then decide whether regular or primary models are needed.'))}</p>
    {mode !== 'local_rules' && <>
      <div className="decision-selector"><DecisionIcon provider={selected?.connection.kind === 'system_one' ? selected.connection.provider : undefined} language={language} /><label className="field"><span className="field-label">{mode === 'custom' ? t('使用的扩展', 'Extension') : t('使用的决策模型', 'Decision model')}</span><select className="input" aria-invalid={!!error} value={selected?.name ? selected.id : ''} onChange={event => { const service = choices.find(item => item.id === event.target.value); if (service) onChange({ kind: 'decision_service', service: structuredClone(service) }); }}><option value="">{t('请选择已保存的连接', 'Select a saved connection')}</option>{selected?.name && !choices.some(item => item.id === selected.id) && <option value={selected.id}>{selected.name}</option>}{choices.map(item => <option value={item.id} key={item.id}>{item.name}</option>)}</select></label></div>
      {selected?.name && <div className="decision-selection-meta"><span className="field-help">{providerLabel(selected.connection, language)} · {t(`配置版本 ${selected.revision}，随路由发布固定`, `Configuration ${selected.revision}, pinned on publication`)}</span>{latest && latest.revision > selected.revision && <button type="button" className="btn btn-quiet" onClick={() => onChange({ kind: 'decision_service', service: structuredClone(latest) })}>{t(`更新到版本 ${latest.revision}`, `Update to version ${latest.revision}`)}</button>}</div>}
      {onOpenServices && <button className="btn btn-quiet" type="button" onClick={() => onOpenServices(mode, service => onChange({ kind: 'decision_service', service: structuredClone(service) }))}><UiIcon name="plus" />{mode === 'custom' ? t('接入自定义扩展', 'Connect custom extension') : t('添加决策模型', 'Add decision model')}</button>}
    </>}
    {error && <p className="oc-inline-error" role="alert">{error}</p>}
  </section>;
}
