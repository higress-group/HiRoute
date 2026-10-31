import { useRef, useState } from 'react';
import { Dialog } from '../../ui';
import type { Draft, Plan } from '../../plan-editor';
import { planErrorCode } from '../../plan-editor-errors';
import type { DecisionServiceView } from './contracts';
import { recordSharedSource } from './form-state';
import { decisionReferences } from './presentation';

export function DecisionRemovalDialog({ target, current, language, mutable, busy, plans, drafts, onConfirm, onClose, onOpenRoute }: {
  target: DecisionServiceView; current?: DecisionServiceView; language: 'zh' | 'en'; mutable: boolean; busy: boolean;
  plans: Plan[]; drafts: Draft[]; onConfirm(target: DecisionServiceView): Promise<void>; onClose(): void;
  onOpenRoute?(plan?: Plan, draft?: Draft): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [error, setError] = useState<{ message: string; referenced: boolean } | null>(null);
  const submitting = useRef(false);
  const native = target.service.connection.kind !== 'custom';
  const changed = current?.service.revision !== target.service.revision;
  const references = decisionReferences(target.service.id, plans, drafts);
  async function confirm() {
    if (!mutable || busy || changed || error || references.length || submitting.current) return;
    submitting.current = true;
    try { await onConfirm(target); }
    catch (cause) {
      const referenced = planErrorCode(cause) === 'DECISION_IN_USE';
      setError({ referenced, message: referenced
        ? t('仍被引用，请先调整引用再移除。已停用的路由和保留的历史版本也会阻止移除。', 'Still referenced. Update references before removing it. Disabled routes and retained historical versions also prevent removal.')
        : t('移除未完成。配置或引用可能已变化，请关闭后重新打开重试。', 'Removal did not complete. Configuration or references may have changed. Close and reopen to retry.') });
    } finally { submitting.current = false; }
  }
  return <Dialog open title={t(`移除「${target.service.name}」？`, `Remove “${target.service.name}”?`)} closeLabel={t('关闭', 'Close')} closeDisabled={busy} onClose={onClose}
    footer={<><button className="btn" type="button" disabled={busy} onClick={onClose}>{t('取消', 'Cancel')}</button><span className="mc-footer-spacer" /><button className="btn btn-danger" type="button" disabled={!mutable || busy || changed || !!error || references.length > 0} onClick={() => void confirm()}>{busy ? t('正在保存…', 'Saving…') : native ? t('移除模型', 'Remove model') : t('移除扩展', 'Remove extension')}</button></>}>
    <p>{recordSharedSource(target.service)
      ? t('仅移除这个决策模型，保留 API 接入及其 Key。', 'Remove only this decision model; retain its API connection and keys.')
      : native ? t('将移除这个决策模型配置，历史使用记录保留。', 'Remove this decision model configuration; usage history is retained.')
        : t('将移除这个自定义扩展配置，历史使用记录保留。', 'Remove this custom extension configuration; usage history is retained.')}</p>
    <p className="field-help">{t('被路由、草稿或保留的历史版本引用的配置无法移除。', 'A configuration referenced by routes, drafts or retained historical versions cannot be removed.')}</p>
    {!!references.length && <div className="callout warn" role="alert"><div><strong>{t('仍被引用，请先调整引用再移除。', 'Still referenced. Update references before removing it.')}</strong>{references.map(reference => <button key={reference.key} className="btn btn-quiet" type="button" disabled={busy || !onOpenRoute} onClick={() => { onClose(); onOpenRoute?.(reference.plan, reference.draft); }}>{reference.name || t('未命名草稿', 'Untitled draft')}{reference.draft ? t('（草稿）', ' (draft)') : ''}</button>)}</div></div>}
    {changed && <p className="callout warn" role="alert">{t('这份配置已发生变化，请关闭后重新打开，确认当前配置。', 'This configuration changed. Close and reopen to review the current configuration.')}</p>}
    {error && <div className={'callout ' + (error.referenced ? 'warn' : 'bad')} role="alert">{error.message}</div>}
  </Dialog>;
}
