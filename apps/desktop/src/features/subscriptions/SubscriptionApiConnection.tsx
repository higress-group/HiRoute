import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { Disclosure } from '../../ui';
import { invoke } from '../../host/index.ts';
import { modelConnectionBackend } from '../model-connections/backend';
import { NativeNetworkSettings } from '../model-connections/NativeNetworkSettings';
import { manualProxyError, networkDraft, networkFailureText, networkRepairable } from '../model-connections/native-network';
import type { ComputeSaveResult, OperationReference } from '../model-connections/types';
import type { ManagedSource, ManagementSnapshot } from '../models/types';
import { apiLinkItems, associatedApiOption, subscriptionProvider, type LinkedSubscriptionProvider, type ApiSyncResult } from './api-sync';
import { NEW_SUBSCRIPTION_API, NewSubscriptionApiSetup, type InlineApiResult } from './new-api';
import { SubscriptionApiCredits } from './SubscriptionApiCredits';
import './subscription-api.css';

export type SubscriptionApiAssociate = (sourceId: string, apiSourceId: string, expectedRevision?: number, current?: () => boolean) => Promise<'saved' | 'uncertain'>;
type Common = {
  language: 'zh' | 'en'; snapshot: ManagementSnapshot; mutable: boolean; busy?: boolean;
  onRefresh(): Promise<void>; onOperation(operation: OperationReference): void; onUncertain(): void;
  onAssociate: SubscriptionApiAssociate;
};
export type SubscriptionApiPickerHandle = {
  validate(): boolean;
  connect(source: Pick<ManagedSource, 'source_id' | 'revision'>): Promise<InlineApiResult>;
};

export function inlineApiResultText(result: InlineApiResult, zh: boolean): string {
  if (result.reason === 'readback_failed') return zh ? '订阅与 API 已保存，暂时无法读取最终状态。刷新只读取状态，不会重复保存。' : 'The subscription and API are saved. The final status is unavailable. Refresh only reads the status and does not save again.';
  if (result.state === 'pending') return zh ? 'API 保存结果待确认。可刷新原操作结果；订阅已保留。' : 'The API save is awaiting confirmation. Refresh the original operation; the subscription is retained.';
  const copy = {
    key_required: ['请填写 API Key，或选择仅使用订阅。', 'Enter an API key, or choose subscription only.'],
    unavailable: ['API 配置信息暂不可用，请重试。订阅已保留。', 'API configuration is unavailable. Retry; the subscription is retained.'],
    check_failed: ['API 检查未完成，请检查 Key、权限或网络后重试。订阅已保留。', 'The API check did not complete. Review the key, permissions or network and retry. The subscription is retained.'],
    models_unavailable: ['API 暂无可接入的同 ID 模型。订阅已保留，可稍后更换 API 或重试。', 'The API has no connectable exact-ID counterparts. The subscription is retained; choose another API or retry later.'],
    save_failed: ['API 未保存，请重新填写 Key 后重试。订阅已保留。', 'The API was not saved. Re-enter the key and retry; the subscription is retained.'],
    link_failed: ['API 已保存，关联尚未完成。重试将复用这份 API 接入。', 'The API was saved, but its association is incomplete. Retry reuses that API connection.'],
    source_changed: ['订阅或 API 配置已变化，请关闭后重新打开确认。已有接入均已保留。', 'The subscription or API changed. Close and reopen to review. Existing connections are retained.'],
  } as const;
  return result.reason ? copy[result.reason][zh ? 0 : 1] : '';
}

export const SubscriptionApiPicker = forwardRef<SubscriptionApiPickerHandle, Common & {
  value: string; provider: LinkedSubscriptionProvider; onChange(value: string): void;
}>(function SubscriptionApiPicker(props, ref) {
  const zh = props.language === 'zh', providerName = props.provider === 'codex' ? 'OpenAI' : 'Anthropic';
  const latest = useRef(props); latest.current = props;
  const alive = useRef(true), key = useRef<HTMLInputElement>(null);
  const feedback = useRef<HTMLParagraphElement>(null);
  const [network, setNetwork] = useState(networkDraft);
  const [reveal, setReveal] = useState(0);
  const [result, setResult] = useState<InlineApiResult | null>(null);
  const [keyRequired, setKeyRequired] = useState(false);
  const [setup] = useState(() => new NewSubscriptionApiSetup(props.provider, {
    backend: modelConnectionBackend(props.language),
    read: () => invoke<ManagementSnapshot>('compute_management_snapshot'),
    find: async idempotencyKey => {
      const value = await invoke<ComputeSaveResult>('get_compute_save_result', { operation: null, idempotencyKey });
      return value.reason === 'OPERATION_NOT_FOUND' ? null : value;
    },
    associate: (...args) => latest.current.onAssociate(...args),
    onOperation: operation => latest.current.onOperation(operation),
    onUncertain: () => latest.current.onUncertain(),
    current: () => alive.current && latest.current.mutable && latest.current.value === NEW_SUBSCRIPTION_API,
  }));
  useEffect(() => { alive.current = true; return () => { alive.current = false; void setup.dispose(); }; }, [setup]);
  useEffect(() => { if (props.value !== NEW_SUBSCRIPTION_API) { void setup.dispose(); setResult(null); } }, [props.value, setup]);
  useEffect(() => {
    if (!result || result.state === 'saved' || props.busy || networkRepairable(result.failure)) return;
    if (result.failure?.phase === 'upstream_auth' || result.reason === 'key_required') key.current?.focus();
    else feedback.current?.focus();
  }, [result, props.busy]);
  useImperativeHandle(ref, () => ({
    validate() {
      if (props.value !== NEW_SUBSCRIPTION_API) return true;
      if (manualProxyError(network)) { setReveal(value => value + 1); return false; }
      if (setup.awaitingSave || setup.hasSavedApi || setup.hasCredential || key.current?.value.trim()) return true;
      setKeyRequired(true); key.current?.focus(); return false;
    },
    async connect(source) {
      const secret = key.current?.value ?? '';
      const answer = await setup.connect(source, secret, network, () => { if (key.current?.value === secret) key.current.value = ''; });
      if (alive.current) { setResult(answer); if (networkRepairable(answer.failure)) setReveal(value => value + 1); }
      return answer;
    },
  }));
  const choices = props.snapshot.sources.filter(source => source.provenance !== 'connector_owned');
  const preferred = choices.filter(source => source.connection_identity?.connection_option_id === associatedApiOption(props.provider));
  const others = choices.filter(source => !preferred.includes(source));
  const usingExisting = props.value !== '' && props.value !== NEW_SUBSCRIPTION_API;
  const option = (source: ManagedSource) => <option key={source.source_id} value={source.source_id}>{source.display_name}{source.state !== 'ready' ? (zh ? '（当前不可用）' : ' (unavailable)') : ''}</option>;
  const locked = !props.mutable || props.busy || setup.awaitingSave;
  function subscriptionOnly() {
    if (locked) return;
    if (key.current) key.current.value = '';
    void setup.dispose(); setResult(null); setKeyRequired(false); props.onChange('');
  }
  return <section className="detail-section subscription-api-setup" data-subscription-api-setup>
    <p className="field-help">{usingExisting
      ? (zh ? '沿用订阅中已选的模型，向下方已有 API 接入同步导入，无需重复选择。' : 'Reuse the subscription’s model selection in the existing API connection below.')
      : setup.hasCredential && !setup.hasSavedApi
        ? (zh ? 'Key 已暂存，可重试或更换；清除 Key 后可仅使用订阅。订阅已选模型会沿用，无需重复选择。' : 'The key is retained for retry or replacement. Clear it to use only the subscription. Its model selection is reused.')
      : (zh ? 'Key 留空则仅使用订阅。填写后沿用订阅已选模型，无需重复选择；同一个保存按钮完成接入，API 失败时订阅仍会保留。' : 'Leave the key blank to use only the subscription. Enter a key to reuse its selected models. One save completes setup; API failures keep the subscription intact.')}</p>
    {result && result.state !== 'saved' && <p ref={feedback} tabIndex={-1} className="callout warn subscription-api-feedback" role="status">{result.failure ? networkFailureText(result.failure, zh) : inlineApiResultText(result, zh)}</p>}
    {setup.hasSavedApi && result?.state !== 'saved' && <p className="field-help">{zh ? '这份 API 接入已独立保存，重试关联不会重复创建。' : 'This API connection is saved independently. Retrying the association will not create another.'}</p>}
    {!usingExisting && <div className="subscription-api-key">
      <label className="field"><span className="field-label">{providerName} API Key{zh ? '（可选）' : ' (optional)'}</span>
        <input ref={key} className="input" type="password" autoComplete="new-password" spellCheck={false} data-subscription-api-key disabled={locked || setup.hasSavedApi} aria-invalid={keyRequired || undefined} placeholder={setup.hasCredential ? (zh ? 'Key 已暂存，重新输入可替换' : 'Key retained; enter a new key to replace it') : (zh ? '留空则仅使用订阅' : 'Leave blank for subscription only')} onChange={event => {
          setKeyRequired(false); setResult(null);
          if (!setup.hasCredential && !setup.hasSavedApi && !setup.awaitingSave) props.onChange(event.target.value.trim() ? NEW_SUBSCRIPTION_API : '');
        }} />
        {keyRequired && <span className="field-help" role="alert">{zh ? '请填写 Key，或选择仅使用订阅。' : 'Enter a key, or choose subscription only.'}</span>}
        <span className="field-help">{zh ? '如使用订阅 API 赠金，请填写接收赠金的 API 组织下的 Key。' : 'To use subscription API credits, use a key from the organization receiving those credits.'}</span>
      </label>
      {setup.hasCredential && !setup.hasSavedApi && <button type="button" className="btn btn-quiet" disabled={locked} onClick={subscriptionOnly}>{zh ? '清除 Key，仅使用订阅' : 'Clear key; use subscription only'}</button>}
      <NativeNetworkSettings language={props.language} value={network} onChange={setNetwork} disabled={locked || setup.hasSavedApi} reveal={reveal} failure={result?.failure}
        target={{ kind: 'registered', connection_option_id: associatedApiOption(props.provider), protocol: props.provider === 'codex' ? 'responses' : 'messages' }} />
    </div>}
    {(choices.length > 0 || usingExisting) && <Disclosure className="subscription-api-existing" language={props.language} defaultOpen={usingExisting} label={zh ? '使用已有 API 接入' : 'Use an existing API connection'}>
      <label className="field"><span className="field-label">{zh ? '已保存的 API 接入' : 'Saved API connection'}</span>
        <select className="select" data-subscription-api-choice disabled={locked} value={usingExisting ? props.value : ''} onChange={event => { setResult(null); props.onChange(event.target.value); }}>
          <option value="">{zh ? '不使用已有接入' : 'Do not use an existing connection'}</option>
          {usingExisting && !choices.some(source => source.source_id === props.value) && <option value={props.value}>{zh ? '关联的 API 接入已删除' : 'Associated API connection was deleted'}</option>}
          {preferred.length > 0 && <optgroup label={zh ? `已有 ${providerName} API 接入` : `Existing ${providerName} API connections`}>{preferred.map(option)}</optgroup>}
          {others.length > 0 && <optgroup label={zh ? '其他已有 API 接入' : 'Other existing API connections'}>{others.map(option)}</optgroup>}
        </select>
      </label>
      {usingExisting && <button className="btn btn-quiet" type="button" disabled={locked} onClick={subscriptionOnly}>{zh ? `改填 ${providerName} API Key` : `Enter a ${providerName} API key instead`}</button>}
    </Disclosure>}
    <SubscriptionApiCredits provider={props.provider} language={props.language} />
  </section>;
});

export function SavedSubscriptionApi(props: Common & {
  source: ManagedSource; result?: ApiSyncResult;
  onRetry(): void;
}) {
  const zh = props.language === 'zh';
  const link = props.snapshot.subscription_api_links?.find(item => item.source_id === props.source.source_id);
  const api = props.snapshot.sources.find(source => source.source_id === link?.api_source_id);
  const picker = useRef<SubscriptionApiPickerHandle>(null);
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
      const choice = selection ?? link?.api_source_id ?? '';
      if (choice === NEW_SUBSCRIPTION_API) {
        if (!picker.current?.validate()) return;
        const answer = await picker.current.connect(props.source);
        if (answer.state !== 'saved') return;
      }
      const outcome = choice === NEW_SUBSCRIPTION_API ? 'saved' : await props.onAssociate(props.source.source_id, choice, props.source.revision);
      setEditing(false); setSelection(undefined);
      if (outcome === 'uncertain') setError(zh ? '关联保存结果待确认，请等待操作恢复后刷新。' : 'Association save is awaiting confirmation. Refresh after operation recovery.');
    } catch { setError(zh ? '关联未保存，请刷新后重试。订阅和 API 模型均保留。' : 'The association was not saved. Refresh and retry; subscription and API models are retained.'); }
    finally { setSaving(false); }
  }
  return <section className="detail-section">
    <div className="detail-section-head"><h3>{zh ? '订阅与 API' : 'Subscription and API'}</h3><button className="btn btn-quiet" type="button" disabled={!props.mutable || props.busy || saving} onClick={() => setEditing(!editing)}>{link ? (zh ? '调整关联' : 'Edit association') : (zh ? '关联 API 接入' : 'Associate an API connection')}</button></div>
    {link && <>
      <p>{api?.display_name ?? (zh ? '关联的 API 接入已删除，请调整关联。' : 'The associated API connection was deleted. Edit the association.')}</p>
      {api && api.state !== 'ready' && <p className="field-help" role="status">{zh ? '此 API 接入当前不可用。请在 API 接入中启用或补充凭据；已导入的模型会保留。' : 'This API connection is unavailable. Enable it or provide credentials in the API connection; imported models are retained.'}</p>}
      <p role="status">{props.busy ? (zh ? '正在同步 API 模型…' : 'Syncing API models…') : waiting.length ? (zh ? `${items.length - waiting.length} 个已导入，${waiting.length} 个待处理。订阅可独立使用。` : `${items.length - waiting.length} imported, ${waiting.length} need attention. The subscription remains independent.`) : (zh ? '模型已同步导入；路由候选组仍由你选择。' : 'Models imported in both sources. You still choose routing candidate groups.')}</p>
      {!!waiting.length && <><Disclosure language={props.language} label={zh ? '查看待同步模型' : 'View pending models'}><ul>{waiting.map(item => <li key={item.model_id}>{item.model_id} · {item.state === 'unsupported' ? (zh ? 'API 目录不提供此模型' : 'Not offered by the API directory') : (zh ? '待同步' : 'Pending sync')}</li>)}</ul></Disclosure>
        <button className="btn" type="button" disabled={!props.mutable || props.busy || saving || !api} onClick={props.onRetry}>{zh ? '重试同步' : 'Retry sync'}</button></>}
    </>}
    {editing && <>
      <SubscriptionApiPicker ref={picker} {...props} provider={subscriptionProvider(props.source)!} busy={props.busy || saving} value={selection ?? link?.api_source_id ?? ''} onChange={setSelection} />
      <p className="field-help">{zh ? '取消关联只停止后续同步，保留两边已经导入的模型。' : 'Removing the association stops future syncs and retains models already imported into both sources.'}</p>
      <button className="btn btn-primary" type="button" disabled={!props.mutable || props.busy || saving} onClick={() => void save()}>{saving ? (zh ? '正在保存…' : 'Saving…') : (zh ? '保存设置' : 'Save setup')}</button>
    </>}
    {error && <p className="callout warn" role="status">{error}</p>}
  </section>;
}
