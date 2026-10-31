import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { readCurrentCatalog } from './catalog-read';
import type { ManagedSource, SavePreview } from '../models/types';
import { ModelCapabilityInput } from './ModelCapabilityInput';
import { manualModelError } from '../model-connections/form-support';
import type { ModelDeclaration } from '../model-connections/types';

type Field = 'tool' | 'vision' | 'streaming' | 'context_tokens' | 'max_output_tokens' | 'native_reasoning';
type Preference = { intent: 'custom'; value: unknown } | { intent: 'follow_metadata' };
type FieldView = { saved_value: unknown; effective_value: unknown; intent: string; origin: string; locator: Record<string, string> | null };
type Defaults = { source_id: string; source_revision: number; revisions: { target: number; dependencies: Record<string, number> }; reviewed_on: string; models: { binding_id: string; fields: Record<Field, FieldView>; native_reasoning_reference?: unknown }[] };
const follows = (field: FieldView) => ['follow_metadata', 'follow_official'].includes(field.intent) || (field.intent === 'preserve_saved' && (field.saved_value === null || field.origin !== 'preserved'));

export function ModelDefaultsEditor({ source, bindingId, language, mutable, onApply, onSaved }: {
  source: ManagedSource; bindingId: string; language: 'zh' | 'en'; mutable: boolean;
  onApply(preview: SavePreview): Promise<'saved' | 'uncertain'>; onSaved(): Promise<void>;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const identity = JSON.stringify([source.source_id,bindingId]);
  const current = useRef(identity); current.current = identity;
  const editRevisions = useRef<Defaults['revisions'] | null>(null);
  const saving = useRef(false);
  const [view, setView] = useState<Defaults | null>(null);
  const [draft, setDraft] = useState<Partial<Record<Field, Preference>>>({});
  const [custom, setCustom] = useState<Partial<Record<Field, unknown>>>({});
  const [retry, setRetry] = useState(0);
  const [error, setError] = useState('');
  const [uncertain, setUncertain] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => { saving.current = false; editRevisions.current = null; setDraft({}); setCustom({}); setView(null); setError(''); setBusy(false); setUncertain(false); }, [source.source_id, bindingId]);
  useEffect(() => {
    let live = true;
    const load = () => { void readCurrentCatalog(() => invoke<Defaults>('compute_defaults', { input: { source_id: source.source_id, include_native_reference: true } })).then(value => {
      if (live) {setView(value); setError('');}
    }).catch(() => {if (live) setError(t('未能读取模型资料，请重试。', 'Could not load model metadata. Retry.'));}); };
    load(); window.addEventListener('model-feed-changed', load);
    return () => {live = false; window.removeEventListener('model-feed-changed', load);};
  }, [source.source_id, source.revision, bindingId, retry]);
  const model = view?.models.find(m => m.binding_id === bindingId);
  const labels: Record<Field, string> = language === 'zh' ? {tool:'工具调用', vision:'图片输入', streaming:'流式输出', context_tokens:'上下文上限', max_output_tokens:'输出上限', native_reasoning:'思考设置'} : {tool:'Tools', vision:'Vision', streaming:'Streaming', context_tokens:'Context limit', max_output_tokens:'Output limit', native_reasoning:'Reasoning'};
  const format = (value: unknown): string => value === null ? t('未知', 'Unknown') : typeof value === 'boolean' ? value ? t('支持', 'Supported') : t('不支持', 'Not supported') : typeof value === 'number' ? value.toLocaleString() : typeof value === 'object' ? (() => {
    const native = value as {kind?:string;profile?:string;profiles?:string[]};
    if (native.kind === 'fixed') return native.profile === 'non-thinking' ? t('不使用思考', 'No reasoning') : t('供应商固定', 'Provider fixed');
    if (native.kind === 'toggle') return t('可切换', 'Switchable');
    if (native.kind === 'discrete') return native.profiles?.join(' / ') ?? t('可选档位', 'Profiles');
    if (native.kind === 'budget') return t('可调思考预算', 'Adjustable reasoning budget');
    return t('未知', 'Unknown');
  })() : String(value);
  function valueFor(name: Field, field: FieldView) { const choice = draft[name]; return choice?.intent === 'custom' ? choice.value : field.effective_value; }
  async function save() {
    if (!view || !model || saving.current || busy || uncertain) return;
    const facts = Object.fromEntries(Object.entries(model.fields).map(([key, field]) => {const value = valueFor(key as Field, field);return [key,{value,basis:value === null ? 'unknown' : 'user_declared'}];}));
    if (manualModelError({upstream_model_id:'metadata-model',display_name:'metadata-model',capabilities:facts} as ModelDeclaration)) {
      setError(t('请检查上限和思考设置：上限须为正整数，思考档位或预算须有效。', 'Check limits and reasoning: use positive integer limits and valid native levels or budgets.')); return;
    }
    saving.current = true; setBusy(true); setError('');
    try {
      const preview = await invoke<SavePreview>('preview_compute_save_v3', {
        change: {schema:'hiroute.compute-management-change/v2',subject:{kind:'saved_source',source_id:source.source_id},expected_revisions:editRevisions.current ?? view.revisions,selected_model_refs:[],intent:source.state === 'disabled' ? 'save_disabled' : 'save_ready',key_edits:[],edit:{action:'rename',display_name:source.display_name}},
        defaultPreferences:{[bindingId]:draft},
      });
      const result = await onApply(preview);
      if (current.current !== identity) return;
      if (result === 'saved') {editRevisions.current = null;setDraft({});setCustom({});setRetry(n => n+1);await onSaved();}
      else setUncertain(true);
    } catch {if (current.current === identity) setError(t('保存未完成，输入已保留。请重新读取资料后重试。', 'Save did not complete. Your input is preserved. Reload metadata and retry.'));}
    finally {if (current.current === identity) {saving.current=false;setBusy(false);}}
  }
  return <section className="detail-section model-defaults-editor">
    <h3>{t('模型能力', 'Model capabilities')}</h3>
    <p className="field-help">{t('跟随 HiRoute 模型资料，或自行修改并保存。仅影响之后的检查与发布。', 'Follow HiRoute model metadata or save your own settings. Changes apply to later checks and publications.')}</p>
    {model && <><p className="field-help">{t('资料日期：'+view?.reviewed_on,'Metadata as of: '+view?.reviewed_on)}</p><fieldset disabled={!mutable || busy || uncertain}><dl>{Object.entries(model.fields).map(([key,field]) => {
      const name=key as Field,choice=draft[name],mode=choice?.intent ?? (follows(field)?'follow_metadata':'custom'),value=valueFor(name,field);
      const origin=mode==='follow_metadata'&&value===null?t('暂无可用模型资料','No usable model metadata'):['custom','preserved'].includes(field.origin)?t('已保存配置','Saved configuration'):field.origin==='unavailable'?t('暂无匹配资料','No matching metadata'):field.origin==='inferred'?t('模型资料推断值','Inferred model metadata'):field.origin==='website_cache'?t('已更新的模型资料','Updated model metadata'):t('内置模型资料','Bundled model metadata');
      return <div className="model-default-field" key={bindingId+'/'+view?.source_revision+'/'+name} data-default-field={name}><dt><label htmlFor={'model-default-'+name}>{labels[name]}</label></dt><dd>
        {mode==='follow_metadata'&&<strong>{format(value)}</strong>}<span className="field-help">{choice?t('尚未保存','Unsaved'):origin}</span>
        <select id={'model-default-'+name} className="select" value={mode} onChange={e=>{
          const policy:Preference=e.target.value==='custom'?{intent:'custom',value:Object.prototype.hasOwnProperty.call(custom,name)?custom[name]:field.effective_value}:{intent:'follow_metadata'};
          editRevisions.current ??= view?.revisions ?? null;setDraft(d=>({...d,[name]:policy}));setError('');
        }}><option value="follow_metadata">{t('跟随 HiRoute 模型资料','Follow HiRoute model metadata')}</option><option value="custom">{t('自定义','Custom')}</option></select>
        {mode==='custom'&&<ModelCapabilityInput field={name} label={labels[name]} value={value} reference={name==='native_reasoning' ? model.native_reasoning_reference ?? field.effective_value : field.effective_value} language={language} onChange={next=>{editRevisions.current ??= view?.revisions ?? null;setCustom(d=>({...d,[name]:next}));setDraft(d=>({...d,[name]:{intent:'custom',value:next}}));setError('');}}/>}
      </dd></div>;
    })}</dl></fieldset></>}
    {error&&<p role="alert">{error} <button className="btn btn-quiet" onClick={()=>{editRevisions.current=null;setRetry(n=>n+1);}} disabled={busy}>{t('重新读取','Reload')}</button></p>}
    {uncertain&&<p role="status">{t('保存结果待确认，请先重新打开此模型查看结果。','Save result is pending. Reopen this model to confirm before saving again.')}</p>}
    {Object.keys(draft).length>0&&<div className="actions"><button className="btn btn-primary" disabled={!mutable||busy||uncertain||!model} onClick={()=>void save()}>{busy?t('正在保存…','Saving…'):t('保存修改','Save changes')}</button></div>}
  </section>;
}
