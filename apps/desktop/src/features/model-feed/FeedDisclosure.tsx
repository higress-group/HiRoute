import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { ModelFeedStatus } from './types';
import './model-feed.css';

/** Shown at the first feed-content entry, using the service's shared acknowledgement. */
export function FeedDisclosure({ language, mutable = true }: { language: 'zh' | 'en'; mutable?: boolean }) {
  const [status, setStatus] = useState<ModelFeedStatus | null>(null);
  const [busy, setBusy] = useState(false), [failed, setFailed] = useState(false);
  const live = useRef(true), pending = useRef(false);
  const zh = language === 'zh';
  const load = useCallback(async () => {
    try { const value = await invoke<ModelFeedStatus>('model_feed_status'); if (live.current) { setStatus(value); setFailed(false); } }
    catch { if (live.current) setFailed(true); }
  }, []);
  useEffect(() => {
    live.current = true; void load(); window.addEventListener('model-feed-changed', load);
    return () => { live.current = false; window.removeEventListener('model-feed-changed', load); };
  }, [load]);
  async function apply(input: object) {
    if (pending.current || !mutable) return;
    pending.current = true; setBusy(true);
    try {
      const value = await invoke<ModelFeedStatus>('model_feed_settings_apply', { input });
      if (live.current) { setStatus(value); setFailed(false); window.dispatchEvent(new Event('model-feed-changed')); }
    } catch { if (live.current) setFailed(true); }
    finally { pending.current = false; if (live.current) setBusy(false); }
  }
  if (status?.disclosure_acknowledged) return null;
  return <aside className="model-feed-disclosure" aria-label={zh ? '模型信息来源' : 'Model information source'}>
    <p>{zh ? '模型信息由 hiroute.ai 提供，自动获取时不发送账户或使用标识。可在这里或模型页的“官网模型目录”中关闭自动获取。' : 'Model information comes from hiroute.ai. Automatic retrieval sends no account or usage identifiers. Turn it off here or under Model directory on the Models page.'}</p>
    <div className="actions"><button className="btn btn-quiet" disabled={busy || !mutable || !status} onClick={() => void apply({ acknowledge_disclosure: true })}>{zh ? '知道了' : 'Got it'}</button>{status?.retrieval_enabled && <button className="btn btn-quiet" disabled={busy || !mutable} onClick={() => void apply({ retrieval_enabled: false, acknowledge_disclosure: true })}>{zh ? '关闭自动获取' : 'Turn off automatic retrieval'}</button>}{failed && <button className="btn btn-quiet" disabled={busy} onClick={() => void load()}>{zh ? '重新读取设置' : 'Reload settings'}</button>}</div>
  </aside>;
}
