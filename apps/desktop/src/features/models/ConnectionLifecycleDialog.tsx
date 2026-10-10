import { useEffect, useRef, useState } from 'react';
import { Dialog } from '../../ui';
import type { ManagedModel, ManagedSource, ManagementChange, ManagementSnapshot, SavePreview } from './types';

type Props = {
  language: 'zh' | 'en';
  source: ManagedSource;
  model?: ManagedModel;
  action: 'rename' | 'remove' | 'delete';
  snapshot: ManagementSnapshot;
  mutable: boolean;
  planNames: Record<string, string>;
  onPreview(change: ManagementChange): Promise<SavePreview>;
  onApply(preview: SavePreview): Promise<'saved' | 'uncertain'>;
  onRefresh(): Promise<void>;
  onClose(): void;
  onOpenPlan?(id: string): void;
};

export function ConnectionLifecycleDialog(props: Props) {
  const { source, model, action } = props;
  const zh = props.language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const rename = action === 'rename';
  const changed = props.snapshot.sources.find(item => item.source_id === source.source_id)?.revision !== source.revision;
  const entire = action === 'delete' || (!rename && source.models.length === 1);
  const [name, setName] = useState(source.display_name);
  const [preview, setPreview] = useState<SavePreview | null>(null);
  const [loading, setLoading] = useState(!rename);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const submitting = useRef(false);
  const change = (displayName = name): ManagementChange => ({
    schema: 'hiroute.compute-management-change/v2',
    subject: { kind: 'saved_source', source_id: source.source_id },
    expected_revisions: props.snapshot.revisions,
    selected_model_refs: !rename && !entire && model ? [model.model_ref] : [],
    intent: source.state === 'disabled' ? 'save_disabled' : 'save_ready',
    key_edits: [],
    edit: rename ? { action: 'rename', display_name: displayName.trim() } : { action: entire ? 'delete' : 'remove_models' },
  });
  useEffect(() => {
    if (rename || changed) return;
    let active = true;
    setLoading(true);
    setPreview(null);
    setError('');
    props.onPreview(change()).then(value => { if (active) setPreview(value); }).catch(() => {
      if (active) setError(text('暂时无法确认引用情况，请刷新后重试。', 'References could not be checked. Refresh and try again.'));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
    // A changed management revision invalidates the preview. Callback identity is not a revision.
  }, [source.source_id, source.revision, model?.model_ref, action, changed, props.snapshot.revisions.target]);
  const references = preview?.affected_plan_refs ?? [];
  async function apply() {
    if (changed || submitting.current || !props.mutable || (!rename && (!preview || references.length))) return;
    if (rename && (!name.trim() || name.trim().length > 60 || /[\u0000-\u001f\u007f]/.test(name)
      || props.snapshot.sources.some(other => other.source_id !== source.source_id && other.display_name.trim() === name.trim()))) {
      setError(text('请输入 1–60 个字符的名称，并与其他接入区分。', 'Use a distinct connection name with 1–60 characters.'));
      return;
    }
    submitting.current = true;
    setSaving(true);
    setError('');
    try {
      const accepted = rename ? await props.onPreview(change()) : preview!;
      await props.onApply(accepted);
      // The host owns pending-operation recovery; an uncertain response must not invite a
      // second delete with a fresh idempotency key.
      props.onClose();
      await props.onRefresh();
    } catch {
      setError(text('未能完成操作。配置或引用可能已变化，请刷新后重试。', 'The operation did not complete. Configuration or references may have changed; refresh and retry.'));
      setPreview(null);
    } finally { submitting.current = false; setSaving(false); }
  }
  const title = rename ? text('重命名接入', 'Rename connection') : entire ? text(`删除「${source.display_name}」？`, `Delete “${source.display_name}”?`) : text(`移除「${model?.display_name}」？`, `Remove “${model?.display_name}”?`);
  return <Dialog open title={title} closeLabel={text('关闭', 'Close')} closeDisabled={saving} onClose={props.onClose}
    footer={<><button className="btn" type="button" disabled={saving} onClick={props.onClose}>{text('取消', 'Cancel')}</button><span className="mc-footer-spacer" /><button className={`btn ${rename ? 'btn-primary' : 'btn-danger'}`} type="button" disabled={changed || !props.mutable || loading || saving || (!rename && (!preview || references.length > 0))} onClick={() => void apply()}>{saving ? text('正在保存…', 'Saving…') : rename ? text('保存名称', 'Save name') : entire ? text('删除接入', 'Delete connection') : text('移除模型', 'Remove model')}</button></>}>
    {rename ? <label className="field"><span className="field-label">{text('接入名称', 'Connection name')}</span><input data-autofocus className="input" maxLength={60} value={name} disabled={saving} onChange={event => setName(event.target.value)} /></label> : <>
      <p>{entire ? text(`将删除这份接入及所属 ${source.models.length} 个模型。`, `Delete this connection and its ${source.models.length} models.`) : text('仅从这份接入移除此模型，保留其他模型及共享凭据。', 'Remove this model from this connection; retain other models and shared credentials.')}</p>
      {entire && <p className="muted">{source.provenance === 'connector_owned' ? text('保留原生客户端登录和配置；再次扫描后需重新添加。', 'Native client sign-ins and configuration are retained. Add the connection again after a new scan.') : text('同时清理这份接入在 HiRoute 中保存的专属凭据。历史使用记录保留。', 'Also remove credentials owned by this HiRoute connection. Usage history is retained.')}</p>}
      {action === 'remove' && entire && <p className="field-help">{text('这是最后一个模型，将一并删除所属接入。', 'This is the last model; its connection will also be deleted.')}</p>}
      {loading && <p role="status">{text('正在检查路由和任务引用…', 'Checking route and task references…')}</p>}
      {!!references.length && <div className="callout warn" role="alert"><div><strong>{text('仍被引用，请先调整引用再删除。', 'Still referenced. Update references before deleting.')}</strong><p>{text('已停用的路由和仍保留执行版本的任务也会阻止删除。', 'Disabled routes and retained execution versions also prevent deletion.')}</p>{references.map(id => <button key={id} className="btn btn-quiet" type="button" disabled={!props.onOpenPlan || !props.planNames[id]} onClick={() => { props.onClose(); props.onOpenPlan?.(id); }}>{props.planNames[id] ?? id}</button>)}</div></div>}
    </>}
    {changed && <p className="callout warn" role="alert">{text('这份接入已发生变化，请关闭后重新打开，确认当前配置。', 'This connection changed. Close and reopen to review its current configuration.')}</p>}
    {error && <div className="callout bad" role="alert">{error}</div>}
  </Dialog>;
}
