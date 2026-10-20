import { readCurrentCatalog } from './catalog-read';
import { ModelMetadataDirectory } from './ModelMetadataDirectory';
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog, UiIcon } from '../../ui';
import { safeDiagnosticCode } from '../../error-code';
import type { FeedRemovalPreview, ModelFeedStatus } from './types';
import './model-feed.css';

export function ModelFeedPanel({ language, mutable = true, onChanged }: { language: 'zh' | 'en'; mutable?: boolean; onChanged?: () => void }) {
  const zh = language === 'zh';
  const [status, setStatus] = useState<ModelFeedStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<FeedRemovalPreview | null>(null);
  const [impactBusy, setImpactBusy] = useState(false);
  async function moreImpact() {
    if (!preview?.next_cursor || impactBusy) return;
    setImpactBusy(true);
    try {const page = await invoke<FeedRemovalPreview>('model_feed_remove_preview', {input:{cursor:preview.next_cursor}}); if (page.digest !== preview.digest) throw new Error('stale'); setPreview({...page, impact:[...preview.impact,...page.impact], sources:[...preview.sources,...page.sources]});}
    catch {setPreview(null);setError(zh ? '影响范围已变化，请重新查看。' : 'Impact changed. Review it again.');}
    finally {setImpactBusy(false);}
  }
  const [disable, setDisable] = useState(true);
  const active = useRef(false);
  const live = useRef(true);
  const load = useCallback(async () => {
    try { const result = await readCurrentCatalog(() => invoke<ModelFeedStatus>('model_feed_status')); if (live.current) { setStatus(result); setError(''); } }
    catch { if (live.current) setError(zh ? '暂时无法读取官网模型目录的状态。' : 'Model directory status is unavailable.'); }
  }, [zh]);
  useEffect(() => { live.current = true; void load(); return () => { live.current = false; }; }, [load]);
  async function change(command: string, input?: unknown) {
    if (active.current || !mutable) return;
    active.current = true; setBusy(true); setError('');
    try {
      const result = await invoke<ModelFeedStatus>(command, input === undefined ? undefined : { input });
      if (live.current) { setStatus(result); setPreview(null); window.dispatchEvent(new Event('model-feed-changed')); onChanged?.(); }
    } catch (failure) {
      if (live.current) {
        setError(safeDiagnosticCode(failure, 'MODEL_FEED_UNAVAILABLE').toLowerCase().includes('stale') ? (zh ? '影响范围已变化，请重新查看后再移除。' : 'The affected sources changed. Review the impact again.') : (zh ? '操作未完成，请重试。已完成内容校验的信息会保留。' : 'The action did not complete. Retry; validated information is retained.'));
        setPreview(null);
      }
    } finally { active.current = false; if (live.current) setBusy(false); }
  }
  async function removePreview() {
    if (active.current || !mutable) return;
    active.current = true; setBusy(true); setError('');
    try { const result = await invoke<FeedRemovalPreview>('model_feed_remove_preview'); if (live.current) { setDisable(true); setPreview(result); } }
    catch { if (live.current) setError(zh ? '无法确认影响范围，请重新读取后再试。' : 'The removal impact could not be verified. Reload and retry.'); }
    finally { active.current = false; if (live.current) setBusy(false); }
  }
  const refusal = status?.verification.kind === 'discarded' ? status.verification.reason : null;
  const refusalText = refusal === 'feed_state_invalid' ? (zh ? '保存的更新状态已损坏。' : 'Saved feed state is damaged.')
    : refusal === 'feed_state_newer_schema' ? (zh ? '保存的状态来自更新版本，请使用相应版本打开。' : 'Saved state uses a newer format. Open it with a compatible client.')
    : refusal === 'feed_activation_requires_restart' ? (zh ? '更新未能完成激活。当前目录仍可使用，请重启后恢复更新。' : 'Activation did not finish. The previous catalog remains usable; restart to resume updates.')
    : refusal ? (zh ? '无法安全修改保存的更新状态。请保留现有文件并检查本地存储。' : 'Saved feed state cannot be safely changed. Preserve the files and check local storage.') : '';
  const [open, setOpen] = useState(false);
  const day = (value: number | string | undefined) => value === undefined ? '' : new Date(value).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US', { month: zh ? 'numeric' : 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false });
  const resultLabel: Record<string, string> = zh ? { ok: '已更新', not_modified: '没有新内容', already_current: '已是最新', not_published: '暂无发布', network: '获取失败', size: '内容超出限制', schema: '格式不支持', validation: '内容校验失败', older_sequence: '已拒绝旧目录' } : { ok: 'Updated', not_modified: 'Nothing new', already_current: 'Already current', not_published: 'Nothing published yet', network: 'Retrieval failed', size: 'Content exceeds limit', schema: 'Unsupported format', validation: 'Content validation failed', older_sequence: 'Older directory rejected' };
  const failedCheck = status?.last_attempt && !['ok', 'not_modified', 'already_current', 'not_published'].includes(status.last_attempt.result);
  const summary = !status ? (error ? '' : zh ? '正在读取…' : 'Loading…')
    : [
      'hiroute.ai',
      status.held ? (zh ? `更新于 ${day(status.held.issued_at)}` : `Updated ${day(status.held.issued_at)}`) : (zh ? '还没有获取到目录' : 'No directory retrieved yet'),
      status.verification.kind === 'validated' ? (zh ? '内容已校验' : 'Content validated') : status.verification.kind === 'discarded' ? (zh ? '目录未被采用' : 'Directory not used') : '',
      status.held ? (zh ? '离线时使用本机缓存' : 'Cached for offline use') : '',
      failedCheck ? (zh ? '上次检查失败' : 'Last check failed') : '',
    ].filter(Boolean).join(' · ');
  return <section className="model-feed-panel" aria-label={zh ? '官网模型目录' : 'Model directory'}>
    <div className="model-feed-bar">
      <div className="model-feed-title"><strong>{zh ? '官网模型目录' : 'Model directory'}</strong><span>{summary}</span></div>
      {status && <div className="model-feed-actions">
        <label className="model-feed-toggle"><input type="checkbox" checked={status.retrieval_enabled} disabled={busy || !mutable || !!refusal} onChange={event => void change('model_feed_settings_apply', { retrieval_enabled: event.target.checked })} /><span>{zh ? '自动获取' : 'Automatic'}</span></label>
        <button className="btn btn-quiet" disabled={busy || !mutable || !!refusal} onClick={() => void change('model_feed_refresh')}>{busy ? (zh ? '处理中…' : 'Working…') : (zh ? '刷新' : 'Refresh')}</button>
        <button className="btn btn-quiet" aria-expanded={open} onClick={() => setOpen(value => !value)}>{zh ? '详情' : 'Details'}<UiIcon name="chevron" /></button>
      </div>}
    </div>
    {status && !status.disclosure_acknowledged && <div className="model-feed-disclosure"><p>{zh ? '模型信息由 hiroute.ai 提供，自动获取时不发送账户或使用标识。你可以在这里关闭自动获取。' : status.disclosure}</p><button className="btn btn-quiet" disabled={busy || !mutable} onClick={() => void change('model_feed_settings_apply', { acknowledge_disclosure: true })}>{zh ? '知道了' : 'Got it'}</button></div>}
    {refusalText && <p className="callout warn" role="status">{refusalText}{status?.recovery_available ? (zh ? '可确认重置缓存，损坏的文件会保留。' : 'You can confirm a cache reset; the damaged file is preserved.') : ''}</p>}
    {error && <p role="alert" className="field-error">{error} <button className="btn btn-quiet" disabled={busy} onClick={() => void load()}>{zh ? '重新读取' : 'Reload'}</button></p>}
    {status && (open || status.recovery_available) && <div className="model-feed-details">
      <p>{zh ? '官网资料提供模型能力、思考设置、价格和证据来源。用户保留的配置优先；内置资料用于离线兜底。更新会用于之后的检查和发布，已有路由和会话保留执行配置。' : 'The directory supplies capabilities, reasoning, prices and sources. Preserved user settings take priority; embedded information provides an offline fallback. Updates apply to later checks and publications; existing routes and sessions retain their execution settings.'}</p>
      <p className="field-help">{zh ? `${status.counts.descriptive_records} 条资料 · ${status.counts.qualified_models} 个模型通过本机执行资格检查 · ${status.counts.restricted_models} 个模型仅展示资料` : `${status.counts.descriptive_records} records · ${status.counts.qualified_models} models qualified by this client · ${status.counts.restricted_models} descriptive-only models`}</p>
      {status.last_attempt && <p className="field-help">{zh ? '上次检查' : 'Last checked'} {day(status.last_attempt.at)} · {resultLabel[status.last_attempt.result] ?? (zh ? '检查未完成' : 'Check incomplete')}</p>}
      <ModelMetadataDirectory language={language} />
      <button className="btn btn-quiet" disabled={busy || !mutable || (!status.held && !status.recovery_available)} onClick={() => void removePreview()}>{status.recovery_available ? (zh ? '重置损坏缓存…' : 'Reset damaged cache…') : (zh ? '移除本机缓存…' : 'Remove local copy…')}</button>
    </div>}
    <Dialog open={Boolean(preview)} title={status?.recovery_available ? (zh ? '重置损坏缓存' : 'Reset damaged cache') : (zh ? '移除本机缓存的模型目录' : 'Remove the local model directory')} closeLabel={zh ? '取消' : 'Cancel'} closeDisabled={busy} onClose={() => setPreview(null)} footer={<><button className="btn" disabled={busy} onClick={() => setPreview(null)}>{zh ? '取消' : 'Cancel'}</button><button className="btn btn-primary" disabled={busy || impactBusy || !preview?.complete} onClick={() => preview && void change('model_feed_remove_apply', { accepted_digest: preview.digest, also_disable_retrieval: disable })}>{status?.recovery_available ? (zh ? '确认重置' : 'Confirm reset') : (zh ? '移除' : 'Remove')}</button></>}>
      {status?.recovery_available && <p>{zh ? '将保留损坏的状态文件并新建缓存状态。' : 'The damaged state file will be preserved and cache state recreated.'}</p>}
      <p>{zh ? '移除后，新检查与发布将使用内置资料。下面列出能力、思考设置或价格默认值会变化的模型。用户保留的值与已发布路由不变。' : 'After removal, new checks and publications use embedded information. Models with changed capability, reasoning or price defaults are listed below. Preserved user values and published routes retain their settings.'}</p>
      {preview && <><p>{zh ? `${preview.impact.length} / ${preview.total} 项影响` : `${preview.impact.length} / ${preview.total} affected models`}</p><ul>{preview.impact.map(record => <li key={`${record.source_id}/${record.binding_id}`}><strong>{record.source_name}</strong><p>{record.model_configuration_id}</p><p>{Object.keys(record.changes).join(' · ')}</p></li>)}</ul>{preview.next_cursor && <button className="btn" disabled={impactBusy} onClick={() => void moreImpact()}>{zh ? '继续查看影响' : 'Review more impacts'}</button>}<p>{zh ? `${preview.published_plan_count} 个已发布路由保持可用。` : `${preview.published_plan_count} published routes remain usable.`}</p></>}
      <label className="model-feed-checkbox"><input type="checkbox" checked={disable} disabled={busy} onChange={event => setDisable(event.target.checked)} /><span>{zh ? '同时关闭自动获取' : 'Also turn off automatic retrieval'}</span></label>
    </Dialog>
  </section>;
}
