import { useState } from 'react';
import { BrandIcon } from './BrandIcon';
import { UiIcon } from './UiIcon';
import { connectionTemplate } from './provider-identity';

const assets = import.meta.glob('./assets/providers/*.svg', { eager: true, query: '?url', import: 'default' }) as Record<string, string>;
export function ProviderIcon({ optionId, providerId, label: customLabel, language }: { optionId?: string | null; providerId?: string; label?: string; language: 'zh' | 'en' }) {
  const provider = connectionTemplate(optionId);
  const [failed, setFailed] = useState('');
  const asset = assets[`./assets/providers/${providerId ?? provider?.provider_id}.svg`];
  const label = customLabel || provider?.name[language] || (language === 'zh' ? 'API 供应商' : 'API provider');
  if (providerId === undefined && optionId === 'codex.subscription.global.v1') {
    return <BrandIcon kind="codex" label={label} className="model-avatar" />;
  }
  return <span className="model-avatar provider-icon" role="img" aria-label={label}>
    {asset && failed !== asset ? <img src={asset} alt="" width="28" height="28" onError={() => setFailed(asset)} /> : <UiIcon name="models" />}
  </span>;
}
