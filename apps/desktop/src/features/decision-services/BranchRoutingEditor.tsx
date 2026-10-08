import { DecisionSelector } from './DecisionSelector';
import type { OpenDecisionConnection } from './presentation';
import { useState, type ReactNode } from 'react';
import { Disclosure, UiIcon } from '../../ui';
import type { Selection } from '../../plan-editor';
import { newBranch, type BranchRouting, type DecisionService, type RouteBranch } from './types';
import { JudgmentFields, judgmentSummary } from './JudgmentSettings';

export type DecisionModelGroup = (title: string, subtitle: string, values: Selection[], replace: (values: Selection[]) => void, options: { groupId: string; preferred?: 'low' | 'high' }) => ReactNode;
export function BranchRoutingEditor({ routing, services, language, onChange, onOpenServices, group, errorGroup, error }: {
  routing: BranchRouting; services: DecisionService[]; language: 'zh' | 'en';
  onChange(value: BranchRouting): void; onOpenServices?: OpenDecisionConnection; errorGroup?: string; error?: string; group: DecisionModelGroup;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [primaryEditors, setPrimaryEditors] = useState<string[]>([]);
  const patch = (value: Partial<BranchRouting>) => onChange({ ...routing, ...value });
  const put = (branch: RouteBranch, value: Partial<RouteBranch>) => patch({ branches: routing.branches.map(b => b.id === branch.id ? { ...b, ...value } : b) });
  return <>
    <DecisionSelector classifier={routing.classifier} smart={false} services={services} language={language} onChange={classifier => patch({ classifier })} onOpenServices={onOpenServices} error={errorGroup === 'classifier' ? error : undefined} />
    {routing.branches.map((branch, index) => {
      const judgment = branch.judgment ?? routing.judgment, dual = branch.primary_candidates.length > 0;
      return <section className="editor-section branch-routing-card" data-branch-id={branch.id} key={branch.id}>
        <div className="editor-section-heading"><div><h3>{branch.name || t('未命名分支', 'Untitled branch')}</h3><p>{dual ? t('先判断任务类别，再在本分支选择常规或主力模型。', 'Classify the task, then choose regular or primary models within this branch.') : t('匹配此类任务后使用常规模型。', 'Use regular models for tasks in this category.')}</p></div>{routing.branches.length > 2 && <button className="icon-btn" type="button" aria-label={t('删除分支', 'Remove branch')} onClick={() => { const branches = routing.branches.filter(b => b.id !== branch.id); patch({ branches, default_branch_id: routing.default_branch_id === branch.id ? branches[0].id : routing.default_branch_id }); }}><UiIcon name="trash" /></button>}</div>
        <div data-route-group={branch.id}><label className="field"><span className="field-label">{t('分支名称', 'Branch name')}</span><input className="input" maxLength={128} data-decision-field={`branch-${index}-name`} value={branch.name} onChange={e => put(branch, { name: e.target.value })} /></label><label className="field"><span className="field-label">{t('任务条件', 'Task condition')}</span><textarea className="input" rows={3} placeholder={t('例如：根据资料起草或改写文章', 'For example: draft or rewrite an article from supplied materials')} data-decision-field={`branch-${index}-condition`} value={branch.condition} onChange={e => put(branch, { condition: e.target.value })} /><span className="field-help">{t('描述任务类别。简单与复杂的判断在下方“判断设置”中调整。', 'Describe the task category. Adjust simple and complex criteria in Judgment settings below.')}</span></label></div>
        {group(t('常规模型', 'Regular models'), t('处理常规任务，按顺序尝试。', 'Handle regular tasks, tried in this order.'), branch.candidates, candidates => put(branch, { candidates }), { groupId: branch.id, preferred: 'low' })}
        {(dual || primaryEditors.includes(branch.id)) ? <>
          {group(t('主力模型', 'Primary models'), t('任务复杂或上一阶段不胜任时使用，按顺序尝试。', 'Used for complex tasks or low competence, tried in this order.'), branch.primary_candidates, primary_candidates => put(branch, { primary_candidates }), { groupId: branch.id + '-primary', preferred: 'high' })}
          <button className="btn btn-quiet" type="button" onClick={() => { put(branch, { primary_candidates: [] }); setPrimaryEditors(ids => ids.filter(id => id !== branch.id)); }}>{t('仅使用常规模型', 'Use regular models only')}</button>
        </> : <button className="btn" type="button" onClick={() => setPrimaryEditors(ids => [...ids, branch.id])}><UiIcon name="plus" />{t('添加主力模型（可选）', 'Add primary models (optional)')}</button>}
        <Disclosure label={t(`判断设置 · ${branch.judgment ? '单独调整' : '使用计划默认'} · ${judgmentSummary(judgment, dual, language)}`, `Judgment settings · ${branch.judgment ? 'Customized' : 'Plan defaults'} · ${judgmentSummary(judgment, dual, language)}`)} language={language}>
          <div className="field-actions"><p className="field-help">{branch.judgment ? t('本分支使用独立的完整判断设置。', 'This branch uses its own complete judgment settings.') : t('跟随计划默认，单独调整后整套设置独立生效。', 'Follows plan defaults. Customize to make all settings independent.')}</p><button className="btn" type="button" onClick={() => put(branch, { judgment: branch.judgment ? null : structuredClone(routing.judgment) })}>{branch.judgment ? t('恢复计划默认', 'Restore plan defaults') : t('单独调整', 'Customize')}</button></div>
          <JudgmentFields value={judgment} onChange={judgment => put(branch, { judgment })} id={`branch-${index}`} dual={dual} disabled={!branch.judgment} language={language} />
        </Disclosure>
      </section>;
    })}
    <section className="editor-section"><button className="btn" type="button" disabled={routing.branches.length >= 16} onClick={() => patch({ branches: [...routing.branches, newBranch(t(`分支 ${routing.branches.length + 1}`, `Branch ${routing.branches.length + 1}`))] })}><UiIcon name="plus" />{t('添加分支', 'Add branch')}</button></section>
    <section className="editor-section"><Disclosure label={t(`计划默认判断设置 · ${judgmentSummary(routing.judgment, true, language)}`, `Plan default judgment · ${judgmentSummary(routing.judgment, true, language)}`)} language={language}><JudgmentFields value={routing.judgment} onChange={judgment => patch({ judgment })} id="global" language={language} /></Disclosure></section>
    <section className="editor-section"><Disclosure label={t('追问偏好与失败处理', 'Follow-up preference and failure handling')} language={language}>
      <label className="field"><span className="field-label">{t('默认分支', 'Default branch')}</span><select className="input" value={routing.default_branch_id} onChange={e => patch({ default_branch_id: e.target.value })}>{routing.branches.map(b => <option key={b.id} value={b.id}>{b.name}</option>)}</select><span className="field-help">{t('没有匹配的任务条件时使用。整体决策失败时也使用此分支，有主力则使用主力。', 'Used when no task condition matches. Decision failures also use this branch, preferring its primary group when present.')}</span></label>
      <FollowUpPreference value={routing.reselect_on_user_message} onChange={reselect_on_user_message => patch({ reselect_on_user_message })} language={language} />
      <p className="field-help">{t('模型不可用时依次尝试本组候选，常规组耗尽后尝试本分支主力组。主力组耗尽后停止并提示。', 'Try candidates in order on failure; exhausted regular groups continue to this branch’s primary group. Stop when primary candidates are exhausted.')}</p>
    </Disclosure></section>
  </>;
}
export function FollowUpPreference({ value, onChange, language }: { value: boolean; onChange(value: boolean): void; language: 'zh' | 'en' }) {
  return <label className="field"><span><input type="checkbox" checked={!value} onChange={e => onChange(!e.target.checked)} />{language === 'zh' ? '优先保持当前模型' : 'Prefer the current model'}</span><span className="field-help">{language === 'zh' ? '每条新消息都会重新决策，此偏好只在本次选中的模型组内生效。' : 'Every new message gets a fresh decision. This preference applies only within the selected model group.'}</span></label>;
}
