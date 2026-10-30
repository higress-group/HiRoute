import { useRef, useState } from 'react';
import { Disclosure } from '../../ui';
import { ModelConnectionForm } from '../model-connections/ModelConnectionForm';
import { modelConnectionBackend } from '../model-connections/backend';
import { newModelConnectionDraft } from '../model-connections/new-draft';
import type { OperationReference } from '../model-connections/types';
import type { ManagedSource, ManagementSnapshot } from '../models/types';
import { apiLinkItems, type ApiSyncResult } from './api-sync';

type Common = {
  language: 'zh' | 'en'; snapshot: ManagementSnapshot; mutable: boolean; busy?: boolean;
  onRefresh(): Promise<void>; onOperation(operation: OperationReference): void; onUncertain(): void;
};
export function SubscriptionApiPicker(props: Common & { value: string; modelIds: string[]; onChange(value: string): void }) {
  const zh = props.language === 'zh';
  const [creating, setCreating] = useState(false);
  const draft = useRef(newModelConnectionDraft('known'));
  const choices = props.snapshot.sources.filter(source => source.provenance !== 'connector_owned');
  return <section className="detail-section">
    <label className="field"><span className="field-label">{zh ? '关联 API 接入（可选）' : 'Associated API connection (optional)'}</span>
      <select className="select" disabled={!props.mutable || props.busy} value={props.value} onChange={event => props.onChange(event.target.value)}>
        <option value="">{zh ? '不关联 API' : 'No associated API'}</option>
        {props.value && !choices.some(source => source.source_id === props.value) && <option value={props.value}>{zh ? '关联的 API 接入已删除' : 'Associated API connection was deleted'}</option>}
        {choices.map(source => <option key={source.source_id} value={source.source_id}>{source.display_name}{source.state !== 'ready' ? (zh ? '（当前不可用）' : ' (unavailable)') : ''}</option>)}
      </select>
    </label>
    <p className="field-help">{zh ? '保存订阅后，也会向此 API 接入导入相同 ID 的模型。以后追加导入继续同步；API 失败不影响订阅，可稍后重试。候选组和优先级由你选择。' : 'After saving the subscription, import matching model IDs into this API connection too. Later imports also sync. API failures keep the subscription intact and can be retried. You choose candidate groups and their priority.'}</p>
    <button className="btn btn-quiet" type="button" disabled={!props.mutable || props.busy} onClick={() => setCreating(true)}>{zh ? '新建关联 API 接入' : 'Create an associated API connection'}</button>
    {creating && <ModelConnectionForm language={props.language} mutable={props.mutable}
      initialConnectionOptionId="openai.platform.global.v1"
      initialDraft={draft.current} initialSelectedModelIds={props.modelIds}
      existingNames={props.snapshot.sources.map(source => source.display_name)} expectedRevisions={props.snapshot.revisions}
      connectionChoice={<p className="field-help">{zh ? 'API 接入独立保存并使用自己的 API Key。完成后返回订阅，保存关联。' : 'The API connection is saved separately and uses its own API key. Return to the subscription afterward to save the association.'}</p>}
      backend={modelConnectionBackend(props.language)} onBack={() => setCreating(false)} onCancel={() => setCreating(false)}
      onSaveAccepted={props.onOperation} onSaveUncertain={props.onUncertain}
      onSaveResult={result => { if (result.disposition === 'saved' && result.source_id) { props.onChange(result.source_id); setCreating(false); } void props.onRefresh(); }}
      restoreFocus={() => {}} />}
  </section>;
}

export function SavedSubscriptionApi(props: Common & {
  source: ManagedSource; result?: ApiSyncResult;
  onAssociate(apiSourceId: string): Promise<'saved' | 'uncertain'>;
  onRetry(): void;
}) {
  const zh = props.language === 'zh';
  const link = props.snapshot.subscription_api_links?.find(item => item.source_id === props.source.source_id);
  const api = props.snapshot.sources.find(source => source.source_id === link?.api_source_id);
  const [selection, setSelection] = useState<string | undefined>();
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const result = props.result?.source_revision === props.source.revision && props.result.api_source_id === link?.api_source_id && props.result.api_revision === api?.revision ? props.result : undefined;
  const items = result?.items ?? apiLinkItems(props.source, api);
  const waiting = items.filter(item => item.state !== 'imported');
  async function save() {
    if (saving || props.busy || !props.mutable) return;
    setSaving(true); setError('');
    try {
      const outcome = await props.onAssociate(selection ?? link?.api_source_id ?? '');
      setEditing(false); setSelection(undefined);
      if (outcome === 'uncertain') setError(zh ? '关联保存结果待确认，请等待操作恢复后刷新。' : 'Association save is awaiting confirmation. Refresh after operation recovery.');
    } catch { setError(zh ? '关联未保存，请刷新后重试。订阅和 API 模型均保留。' : 'The association was not saved. Refresh and retry; subscription and API models are retained.'); }
    finally { setSaving(false); }
  }
  return <section className="detail-section">
    <div className="detail-section-head"><h3>{zh ? '关联 API' : 'Associated API'}</h3><button className="btn btn-quiet" type="button" disabled={!props.mutable || props.busy || saving} onClick={() => setEditing(!editing)}>{link ? (zh ? '调整关联' : 'Edit association') : (zh ? '关联 API 接入' : 'Associate an API connection')}</button></div>
    {link && <>
      <p>{api?.display_name ?? (zh ? '关联的 API 接入已删除，请调整关联。' : 'The associated API connection was deleted. Edit the association.')}</p>
      {api && api.state !== 'ready' && <p className="field-help" role="status">{zh ? '此 API 接入当前不可用。请在 API 接入中启用或补充凭据；已导入的模型会保留。' : 'This API connection is unavailable. Enable it or provide credentials in the API connection; imported models are retained.'}</p>}
      <p role="status">{props.busy ? (zh ? '正在同步 API 模型…' : 'Syncing API models…') : waiting.length ? (zh ? `${items.length - waiting.length} 个已导入，${waiting.length} 个待处理。订阅可独立使用。` : `${items.length - waiting.length} imported, ${waiting.length} need attention. The subscription remains independent.`) : (zh ? '模型已同步导入；路由候选组仍由你选择。' : 'Models imported in both sources. You still choose routing candidate groups.')}</p>
      {!!waiting.length && <><Disclosure language={props.language} label={zh ? '查看待同步模型' : 'View pending models'}><ul>{waiting.map(item => <li key={item.model_id}>{item.model_id} · {item.state === 'unsupported' ? (zh ? 'API 目录不提供此模型' : 'Not offered by the API directory') : (zh ? '待同步' : 'Pending sync')}</li>)}</ul></Disclosure>
        <button className="btn" type="button" disabled={!props.mutable || props.busy || saving || !api} onClick={props.onRetry}>{zh ? '重试同步' : 'Retry sync'}</button></>}
    </>}
    {editing && <>
      <SubscriptionApiPicker {...props} busy={props.busy || saving} value={selection ?? link?.api_source_id ?? ''} onChange={setSelection} modelIds={props.source.models.map(model => model.upstream_model_id)} />
      <p className="field-help">{zh ? '取消关联只停止后续同步，保留两边已经导入的模型。' : 'Removing the association stops future syncs and retains models already imported into both sources.'}</p>
      <button className="btn btn-primary" type="button" disabled={!props.mutable || props.busy || saving} onClick={() => void save()}>{saving ? (zh ? '正在保存…' : 'Saving…') : (zh ? '保存关联' : 'Save association')}</button>
    </>}
    {error && <p className="callout warn" role="status">{error}</p>}
  </section>;
}
