import { useEffect, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { readCurrentCatalog } from './catalog-read';
import type { ManagedSource, SavePreview } from '../models/types';

type Field = 'tool' | 'vision' | 'streaming' | 'context_tokens' | 'max_output_tokens' | 'native_reasoning';
type Locator = Record<string, string>;
type Preference = { intent: 'preserve_saved' } | { intent: 'follow_official'; locator: Locator };
type FieldView = { saved_value: unknown; effective_value: unknown; intent: Preference['intent']; origin: string; locator: Locator | null };
type Defaults = { source_id: string; source_revision: number; revisions: { target: number; dependencies: Record<string, number> }; reviewed_on: string; models: { binding_id: string; fields: Record<Field, FieldView> }[] };

export function ModelDefaultsEditor({ source, bindingId, language, mutable, onApply, onSaved }: {
  source: ManagedSource; bindingId: string; language: 'zh' | 'en'; mutable: boolean;
  onApply(preview: SavePreview): Promise<'saved' | 'uncertain'>; onSaved(): Promise<void>;
}) {
  const zh = language === 'zh';
  const [view, setView] = useState<Defaults | null>(null);
  const [draft, setDraft] = useState<Partial<Record<Field, Preference>>>({});
  const [preview, setPreview] = useState<SavePreview | null>(null);
  const [retry, setRetry] = useState(0);
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => { setDraft({}); setPreview(null); setView(null); }, [source.source_id, bindingId]);
  useEffect(() => {
    let live = true;
    const load = () => { void readCurrentCatalog(() => invoke<Defaults>('compute_defaults', { input: { source_id: source.source_id } })).then(value => {
      if (live) {setView(value); setError(false); setPreview(null);}
    }).catch(() => {if (live) setError(true);}); };
    load(); window.addEventListener('model-feed-changed', load);
    return () => {live = false; window.removeEventListener('model-feed-changed', load);};
  }, [source.source_id, source.revision, bindingId, retry]);
  const model = view?.models.find(m => m.binding_id === bindingId);
  const labels: Record<Field, string> = zh ? {tool:'工具调用', vision:'图片输入', streaming:'流式输出', context_tokens:'上下文上限', max_output_tokens:'输出上限', native_reasoning:'思考设置'} : {tool:'Tools', vision:'Vision', streaming:'Streaming', context_tokens:'Context limit', max_output_tokens:'Output limit', native_reasoning:'Reasoning'};
  const format = (value: unknown): string => value === null ? (zh ? '未知' : 'Unknown') : typeof value === 'boolean' ? value ? (zh ? '支持' : 'Supported') : (zh ? '不支持' : 'Not supported') : typeof value === 'number' ? value.toLocaleString() : typeof value === 'object' ? JSON.stringify(value) : String(value);
  async function prepare() {
    if (!view || busy) return;
    setBusy(true); setError(false);
    try {setPreview(await invoke<SavePreview>('preview_compute_save_v3', {
      change: {schema:'hiroute.compute-management-change/v2', subject:{kind:'saved_source',source_id:source.source_id}, expected_revisions:view.revisions, selected_model_refs:[], intent:source.state === 'disabled' ? 'save_disabled' : 'save_ready', key_edits:[], edit:{action:'rename',display_name:source.display_name}},
      defaultPreferences:{[bindingId]:draft},
    }));} catch {setError(true); setPreview(null);} finally {setBusy(false);}
  }
  async function save() {
    if (!preview || busy) return;
    setBusy(true); setError(false);
    try {const result = await onApply(preview); setPreview(null); if (result === 'saved') {setDraft({}); setRetry(n => n+1); await onSaved();}}
    catch {setError(true); setPreview(null);} finally {setBusy(false);}
  }
  return <section className="detail-section model-defaults-editor">
    <h3>{zh ? '能力默认值' : 'Capability defaults'}</h3>
    <p className="field-help">{zh ? '默认值用于之后的检查与发布；已有路由和会话保留原来的执行配置。保留值使用这次接入原先保存的值。' : 'Defaults apply to later checks and publications. Existing routes and sessions retain their execution settings. Preserved values use the facts saved with this connection.'}</p>
    {model && <><p className="field-help">{zh ? `信息核对日期：${view?.reviewed_on}` : `Evidence reviewed: ${view?.reviewed_on}`}</p><dl>{Object.entries(model.fields).map(([key, field]) => {
      const name = key as Field; const selected = draft[name]?.intent ?? field.intent;
      return <div className="model-default-field" key={name}><dt><label htmlFor={`model-default-${name}`}>{labels[name]}</label></dt><dd><strong>{format(field.effective_value)}</strong><span className="field-help">{field.origin === 'preserved' ? (zh ? '接入中保留的值' : 'Preserved in connection') : field.origin === 'website_cache' ? (zh ? '官网 / 本机缓存' : 'Website / local cache') : (zh ? '内置默认值' : 'Embedded default')}</span><select id={`model-default-${name}`} className="input" value={selected} disabled={!mutable || busy} onChange={event => {
        const policy: Preference = event.target.value === 'follow_official' && field.locator ? {intent:'follow_official',locator:field.locator} : {intent:'preserve_saved'};
        setDraft(d => ({...d,[name]:policy})); setPreview(null);
      }}><option value="preserve_saved">{zh ? `保留已保存值：${format(field.saved_value)}` : `Preserve saved: ${format(field.saved_value)}`}</option><option value="follow_official" disabled={!field.locator}>{zh ? '使用官方默认值' : 'Use official default'}</option></select></dd></div>;
    })}</dl></>}
    {error && <p role="alert">{zh ? '信息或预览已变化，请重新读取后重试。' : 'Information or preview changed. Reload and retry.'} <button className="btn btn-quiet" onClick={() => setRetry(n => n+1)} disabled={busy}>{zh ? '重新读取' : 'Reload'}</button></p>}
    {Object.keys(draft).length > 0 && <div className="actions"><button className="btn" disabled={!mutable || busy} onClick={() => void prepare()}>{zh ? '预览修改' : 'Preview changes'}</button>{preview && <button className="btn btn-primary" disabled={!mutable || busy} onClick={() => void save()}>{zh ? '保存默认值选择' : 'Save default choices'}</button>}</div>}
  </section>;
}
