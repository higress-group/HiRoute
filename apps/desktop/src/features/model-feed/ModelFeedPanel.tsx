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
  const [disable, setDisable] = useState(true);
  const active = useRef(false);
  const live = useRef(true);
  const load = useCallback(async () => {
    try { const result = await invoke<ModelFeedStatus>('model_feed_status'); if (live.current) { setStatus(result); setError(''); } }
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
      <p>{zh ? '官网目录只补充模型信息：能力、思考设置和推荐说明。接入地址、认证方式和请求格式只来自 HiRoute 内置的服务目录，官网目录改不了；目录更新也不会改变已保存的接入和已发布的路由。' : 'The directory only adds model information: capabilities, reasoning settings and recommendations. Endpoints, authentication and request formats come only from the service catalog built into HiRoute, and directory updates never change saved connections or published routes.'}</p>
      {status.held && <p className="field-help">{zh ? `已采用 ${status.counts.applied} 个模型 · ${status.counts.ignored_embedded} 个内置目录已包含 · ${status.counts.newer_client} 个需要更新 HiRoute · ${status.counts.unresolved} 个未能匹配` : `${status.counts.applied} models added · ${status.counts.ignored_embedded} already built in · ${status.counts.newer_client} need a newer HiRoute · ${status.counts.unresolved} unresolved`}</p>}
      {status.last_attempt && <p className="field-help">{zh ? '上次检查' : 'Last checked'} {day(status.last_attempt.at)} · {resultLabel[status.last_attempt.result] ?? (zh ? '检查未完成' : 'Check incomplete')}</p>}
      <button className="btn btn-quiet" disabled={busy || !mutable || (!status.held && !status.recovery_available)} onClick={() => void removePreview()}>{status.recovery_available ? (zh ? '重置损坏缓存…' : 'Reset damaged cache…') : (zh ? '移除本机缓存…' : 'Remove local copy…')}</button>
    </div>}
    <Dialog open={Boolean(preview)} title={status?.recovery_available ? (zh ? '重置损坏缓存' : 'Reset damaged cache') : (zh ? '移除本机缓存的模型目录' : 'Remove the local model directory')} closeLabel={zh ? '取消' : 'Cancel'} closeDisabled={busy} onClose={() => setPreview(null)} footer={<><button className="btn" disabled={busy} onClick={() => setPreview(null)}>{zh ? '取消' : 'Cancel'}</button><button className="btn btn-primary" disabled={busy || !preview} onClick={() => preview && void change('model_feed_remove_apply', { accepted_digest: preview.digest, also_disable_retrieval: disable })}>{status?.recovery_available ? (zh ? '确认重置' : 'Confirm reset') : (zh ? '移除' : 'Remove')}</button></>}>
      {status?.recovery_available && <p>{zh ? '将保留损坏的状态文件并新建缓存状态。' : 'The damaged state file will be preserved and cache state recreated.'}</p>}
      <p>{zh ? '这些接入中由官网目录补充的模型将不能再用于检查、保存或发布新的路由。已发布路由及其会话和 Worker 继续运行。' : 'Directory models in these connections will no longer be available for checks, saves or publishing new routes. Published routes, sessions and Workers keep running.'}</p>
      {preview && <>{preview.sources.length ? <ul>{preview.sources.map(source => <li key={source.source_id}><strong>{source.display_name}</strong><span> · {source.model_configuration_ids.length} {zh ? '个模型' : 'models'}</span></li>)}</ul> : <p>{zh ? '没有保存的接入依赖这份目录。' : 'No saved connection depends on this directory.'}</p>}<p>{zh ? `${preview.published_revisions.length} 个已发布路由保持可用。HiRoute 会记住已接受的最新目录，不会退回更旧的目录。` : `${preview.published_revisions.length} published routes remain usable. HiRoute remembers the newest accepted directory and will not fall back to an older one.`}</p></>}
      <label className="model-feed-checkbox"><input type="checkbox" checked={disable} disabled={busy} onChange={event => setDisable(event.target.checked)} /><span>{zh ? '同时关闭自动获取' : 'Also turn off automatic retrieval'}</span></label>
    </Dialog>
  </section>;
}
