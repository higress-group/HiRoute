import { useEffect, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { ModelAvailabilityResult, ModelFeedStatus } from './types';
import { rollingIdentity, shortDate } from './presentation';

export function ModelOriginDetails({ modelId, language }: { modelId: string; language: 'zh' | 'en' }) {
  const [result, setResult] = useState<ModelAvailabilityResult | null>(null);
  const [status, setStatus] = useState<ModelFeedStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const zh = language === 'zh';
  useEffect(() => {
    let live = true;
    const refresh = () => { void Promise.all([invoke<ModelAvailabilityResult>('model_availability'), invoke<ModelFeedStatus>('model_feed_status')])
      .then(([models, feed]) => { if (live) { setResult(models); setStatus(feed); } }).catch(() => { if (live) { setResult(null); setStatus(null); } }); };
    refresh(); window.addEventListener('model-feed-changed', refresh);
    return () => { live = false; window.removeEventListener('model-feed-changed', refresh); };
  }, [modelId]);
  const model = result?.models.find(model => model.model_configuration_id === modelId);
  if (!model) return null;
  const updates = result?.featured.filter(entry => entry.model_configuration_id === modelId && entry.kind === 'model_update') ?? [];
  const hasFeedContent = model.origin.kind === 'feed' || updates.length > 0;
  async function acknowledge() {
    if (busy) return;
    setBusy(true); setError(false);
    try { setStatus(await invoke<ModelFeedStatus>('model_feed_settings_apply', { input: { acknowledge_disclosure: true } })); window.dispatchEvent(new Event('model-feed-changed')); }
    catch { setError(true); }
    finally { setBusy(false); }
  }
  return <section className="detail-section model-origin-details">
    <h3>{zh ? '模型信息来源' : 'Model information origin'}</h3>
    <p>{model.origin.kind === 'embedded' ? (zh ? 'HiRoute 内置目录' : 'Built into HiRoute') : model.origin.kind === 'feed' ? (zh ? `来自 hiroute.ai 模型目录 · ${shortDate(model.origin.included_on, language)} 收录` : `From the hiroute.ai model directory · added ${shortDate(model.origin.included_on, language)}`) : (zh ? '已发布路由中保存的信息' : 'Information saved in published routes')}</p>
    {updates.map(update => <p key={update.id}>{zh ? `更新公告 ${shortDate(update.included_on, language)}` : `Update announced ${shortDate(update.included_on, language)}`} · {update.note[language]}</p>)}
    {rollingIdentity(model.identity_stability) && <p className="field-help">{zh ? '供应商可能更新同一模型名背后的版本。历史评分可能混合多个版本；收录日和公告日都不代表实际执行的模型版本。' : 'The provider may update the model behind this name. Historical scores may mix versions; dates added and announced do not identify the version that executed.'}</p>}
    {model.availability.kind === 'used_by_published_plans' && <p className="field-help">{zh ? `只在 ${model.availability.count} 个已发布路由中保留，不能用于新发布。` : `Kept only in ${model.availability.count} published routes; unavailable for new publication.`}</p>}
    {hasFeedContent && status && !status.disclosure_acknowledged && <div className="model-feed-disclosure"><p>{zh ? '模型信息由 hiroute.ai 自动提供，不发送账户或使用标识。可在模型页的“官网模型目录”中关闭。' : 'Model information is retrieved automatically from hiroute.ai without account or usage identifiers. Turn it off under Model directory on the Models page.'}</p><button className="btn btn-quiet" disabled={busy} onClick={() => void acknowledge()}>{zh ? '知道了' : 'Got it'}</button>{error && <p role="alert">{zh ? '未能保存，请重试。' : 'Could not save. Try again.'}</p>}</div>}
  </section>;
}
