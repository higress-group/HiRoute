import { useEffect, useRef, useState } from 'react';
import { Dialog } from '../../ui';
import type { ModelConnectionCheckView } from '../model-connections/types';
import type { ManagedSource, ManagementChange, RevisionSet, SavePreview } from './types';

export function AppendConnectionModels(props: {
  source: ManagedSource; language: 'zh' | 'en'; mutable: boolean;
  onLoad(source: ManagedSource): Promise<{ checked: ModelConnectionCheckView; revisions: RevisionSet }>;
  onPreview(change: ManagementChange): Promise<SavePreview>;
  onApply(preview: SavePreview): Promise<'saved' | 'uncertain'>;
  onRefresh(): Promise<void>; onClose(): void; onManual?(): void;
}) {
  const zh = props.language === 'zh';
  const [result, setResult] = useState<Awaited<ReturnType<typeof props.onLoad>> | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const submitting = useRef(false);
  useEffect(() => {
    let active = true;
    setLoading(true); setError(''); setResult(null); setSelected(new Set());
    props.onLoad(props.source).then(value => { if (active) setResult(value); }).catch(() => {
      if (active) setError(zh ? '无法读取这份接入的模型。请检查凭据和网络后重试。' : 'Could not read models for this connection. Check credentials and network, then retry.');
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [props.source.source_id, props.source.revision, attempt]);
  const existing = new Set(props.source.models.map(model => model.upstream_model_id));
  const models = result?.checked.candidate.models.filter(model => `${model.display_name} ${model.upstream_model_id}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) ?? [];
  async function save() {
    if (!result || !selected.size || submitting.current || !props.mutable) return;
    submitting.current = true; setSaving(true); setError('');
    try {
      const preview = await props.onPreview({ schema: 'hiroute.compute-management-change/v2',
        subject: { kind: 'candidate', candidate: result.checked.candidate.candidate },
        expected_revisions: result.revisions, selected_model_refs: [...selected],
        intent: props.source.state === 'disabled' ? 'save_disabled' : 'save_ready', key_edits: [],
        edit: { action: 'append_models' }, validation: result.checked.candidate.validation });
      await props.onApply(preview);
      props.onClose();
      await props.onRefresh();
    } catch { setError(zh ? '未能添加模型，已有配置未被替换。请重新读取后重试。' : 'Models could not be added. Existing settings were not replaced. Reload and retry.'); setResult(null); }
    finally { submitting.current = false; setSaving(false); }
  }
  return <Dialog open title={zh ? `向「${props.source.display_name}」添加模型` : `Add models to “${props.source.display_name}”`} closeLabel={zh ? '关闭' : 'Close'} closeDisabled={saving} onClose={props.onClose}
    footer={<><button className="btn" type="button" disabled={saving} onClick={props.onClose}>{zh ? '取消' : 'Cancel'}</button><span className="mc-footer-spacer" /><button className="btn btn-primary" type="button" disabled={!props.mutable || loading || saving || !result || !selected.size} onClick={() => void save()}>{saving ? zh ? '正在添加…' : 'Adding…' : zh ? `添加 ${selected.size} 个模型` : `Add ${selected.size} models`}</button></>}>
    <p>{zh ? `复用此接入的端点和凭据，保留已有 ${props.source.models.length} 个模型。` : `Reuse this connection's endpoints and credentials; retain its ${props.source.models.length} existing models.`}</p>
    {props.source.state === 'disabled' && <p className="field-help">{zh ? '添加后此接入仍保持停用。' : 'This connection will remain disabled after adding models.'}</p>}
    {loading ? <p role="status">{zh ? '正在读取可添加的模型…' : 'Reading available models…'}</p> : <>
      {error && <div className="callout warn" role="alert">{error}<button className="btn" type="button" onClick={() => setAttempt(value => value + 1)}>{zh ? '重新读取' : 'Reload'}</button></div>}
      {result && <><label className="field"><span className="field-label">{zh ? '搜索模型' : 'Search models'}</span><input className="input" value={query} onChange={event => setQuery(event.target.value)} /></label><div className="model-result-list">{models.map(model => <label className="check-row" key={model.model_ref}><input type="checkbox" disabled={saving || existing.has(model.upstream_model_id) || !model.selectable} checked={existing.has(model.upstream_model_id) || selected.has(model.model_ref)} onChange={event => setSelected(current => { const next = new Set(current); if (event.target.checked) next.add(model.model_ref); else next.delete(model.model_ref); return next; })} /><span><strong>{model.display_name}</strong><span className="field-help">{existing.has(model.upstream_model_id) ? zh ? '已添加' : 'Already added' : model.upstream_model_id}</span></span></label>)}</div>{!models.length && <p role="status">{zh ? '没有匹配的可添加模型。' : 'No matching models to add.'}</p>}</>}
      {props.onManual && <button className="btn btn-quiet" type="button" disabled={saving} onClick={props.onManual}>{zh ? '手工添加模型 ID' : 'Add a model ID manually'}</button>}
    </>}
  </Dialog>;
}
