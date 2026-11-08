import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog } from '../../ui';
import type { Draft, Plan } from '../../plan-editor';
import { planErrorCode } from '../../plan-editor-errors';
import type { DecisionRemovalPreview, DecisionRemovalProtection, DecisionServiceView } from './contracts';
import { recordSharedSource } from './form-state';

export function DecisionRemovalDialog({ target, current, language, mutable, busy, plans, drafts, onConfirm, onClose, onOpenRoute }: {
  target: DecisionServiceView; current?: DecisionServiceView; language: 'zh' | 'en'; mutable: boolean; busy: boolean;
  plans: Plan[]; drafts: Draft[]; onConfirm(target: DecisionServiceView): Promise<void>; onClose(): void;
  onOpenRoute?(plan?: Plan, draft?: Draft): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [error, setError] = useState('');
  const [retryable, setRetryable] = useState(false);
  const [preview, setPreview] = useState<DecisionRemovalPreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [generation, setGeneration] = useState(0);
  const submitting = useRef(false);
  const native = target.service.connection.kind !== 'custom';
  const sourceRevisions = (view: DecisionServiceView) => JSON.stringify(view.related_sources.map(source => [source.source_id, source.revision]).sort());
  const changed = !current || current.service.revision !== target.service.revision || current.configuration_digest !== target.configuration_digest || sourceRevisions(current) !== sourceRevisions(target);
  const locked = busy || refreshing;
  const references = preview?.blockers ?? [];
  const reason: Record<DecisionRemovalProtection, string> = {
    current_head: t('当前路由（含已停用）', 'Current route (including disabled)'),
    prepared_version: t('准备中的版本', 'Prepared version'),
    unresolved_hold: t('任务执行或恢复保留', 'Execution or recovery hold'),
    draft: t('草稿', 'Draft'),
    active_publication: t('当前发布', 'Active publication'),
    prepared_publication: t('准备中的发布', 'Prepared publication'),
    lkg_publication: t('最近可用发布', 'Last-known-good publication'),
  };
  useEffect(() => {
    let active = true;
    setLoading(true); setPreview(null); setError(''); setRetryable(false);
    if (changed) { setLoading(false); return; }
    void invoke<DecisionRemovalPreview>('preview_decision_removal', { input: { id: target.service.id, expected_revision: target.service.revision } }).then(value => {
      if (value.id !== target.service.id || value.revision !== target.service.revision || (value.failure !== null && value.failure !== 'DECISION_IN_USE') || (value.failure === null) !== (value.blockers.length === 0)) throw new Error('DECISION_PREVIEW_INVALID');
      if (active) setPreview(value);
    }).catch(() => {
      if (active) { setRetryable(true); setError(t('暂时无法确认引用情况，请重试。未删除任何配置。', 'References could not be checked. Retry; no configuration was deleted.')); }
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [target.service.id, target.service.revision, changed, generation, language]);
  async function refreshReferences() {
    if (!mutable || locked || changed || submitting.current) return;
    submitting.current = true; setRefreshing(true); setError('');
    try {
      await invoke('refresh_route_references', { language });
      setGeneration(value => value + 1);
    } catch {
      setError(t('引用状态暂未更新，请稍后重试。未删除任何配置。', 'References could not be updated. Retry later; no configuration was deleted.'));
    } finally { submitting.current = false; setRefreshing(false); }
  }
  async function confirm() {
    if (!mutable || locked || changed || loading || !preview || error || references.length || submitting.current) return;
    submitting.current = true;
    try { await onConfirm(target); }
    catch (cause) {
      const code = planErrorCode(cause);
      setPreview(null); setRetryable(false);
      if (['DECISION_IN_USE', 'REVISION_CONFLICT', 'DECISION_REVISION_CONFLICT'].includes(code)) {
        setGeneration(value => value + 1);
      } else {
        setError(t('移除结果尚未确认。请等待现有操作完成后关闭并重新打开查看，不要重复提交。', 'Removal is not confirmed. Let the existing operation finish, then close and reopen to check; do not submit again.'));
      }
    } finally { submitting.current = false; }
  }
  return <Dialog open title={t(`移除「${target.service.name}」？`, `Remove “${target.service.name}”?`)} closeLabel={t('关闭', 'Close')} closeDisabled={locked} onClose={onClose}
    footer={<><button className="btn" type="button" disabled={locked} onClick={onClose}>{t('取消', 'Cancel')}</button><span className="mc-footer-spacer" /><button className="btn btn-danger" type="button" disabled={!mutable || locked || changed || loading || !preview || !!error || references.length > 0} onClick={() => void confirm()}>{busy ? t('正在保存…', 'Saving…') : native ? t('移除模型', 'Remove model') : t('移除扩展', 'Remove extension')}</button></>}>
    <p>{recordSharedSource(target.service)
      ? t('仅移除这个决策模型，保留 API 接入及其 Key。', 'Remove only this decision model; retain its API connection and keys.')
      : native ? t('将移除这个决策模型配置，历史使用记录保留。', 'Remove this decision model configuration; usage history is retained.')
        : t('将移除这个自定义扩展配置，历史使用记录保留。', 'Remove this custom extension configuration; usage history is retained.')}</p>
    <p className="field-help">{t('当前路由、草稿、执行保留和已安装发布会阻止移除；普通历史版本保留，但不再保证可恢复执行。', 'Current routes, drafts, execution holds and installed publications prevent removal. Other history is retained for viewing, without a guarantee of restored execution.')}</p>
    {loading && <p role="status">{t('正在检查路由和任务引用…', 'Checking route and task references…')}</p>}
    {!!references.length && <div className="callout warn" role="alert"><div><strong>{t('仍被引用，请先调整引用再移除。', 'Still referenced. Update references before removing it.')}</strong><ul className="decision-removal-blockers">{references.map((reference, index) => {
      const draft = drafts.find(item => item.draft_id === reference.draft_id);
      const plan = !reference.draft_id ? plans.find(item => item.agent_plan_id === reference.plan_id) : undefined;
      const label = draft?.editor.display_name || plan?.desired.display_name || reference.draft_id || reference.plan_id;
      return <li key={`${reference.protection}-${reference.plan_id}-${reference.draft_id}-${reference.revision}-${index}`}><button className="btn btn-quiet" type="button" disabled={locked || !onOpenRoute || (!plan && !draft)} onClick={() => { onClose(); onOpenRoute?.(plan, draft); }}>{label} · v{reference.revision}</button><span className="field-help"> — {reason[reference.protection]}</span></li>;
    })}</ul>{references.some(reference => reference.protection.endsWith('_publication')) && <><p className="field-help">{t('若已调整或删除路由，可更新引用状态后重新确认。', 'After changing or deleting routes, update references and review again.')}</p><button className="btn" type="button" disabled={!mutable || locked || changed} onClick={() => void refreshReferences()}>{refreshing ? t('正在更新…', 'Updating…') : t('更新引用状态', 'Update references')}</button></>}</div></div>}
    {changed && <p className="callout warn" role="alert">{t('这份配置已发生变化，请关闭后重新打开，确认当前配置。', 'This configuration changed. Close and reopen to review the current configuration.')}</p>}
    {error && <div className="callout bad" role="alert">{error}{retryable && !changed && <button className="btn" type="button" disabled={locked} onClick={() => setGeneration(value => value + 1)}>{t('重试检查', 'Retry check')}</button>}</div>}
  </Dialog>;
}
