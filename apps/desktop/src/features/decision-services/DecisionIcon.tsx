import { ProviderIcon } from '../../ui/ProviderIcon';
import { UiIcon } from '../../ui';

export function DecisionIcon({ provider, language }: { provider?: string; language: 'zh' | 'en' }) {
  if (provider?.startsWith('bailian') || provider === 'openrouter') return <ProviderIcon providerId={provider.startsWith('bailian') ? 'bailian' : 'openrouter'} label={provider.startsWith('bailian') ? (language === 'zh' ? '百炼' : 'Bailian') : 'OpenRouter'} language={language} />;
  return <span className="model-avatar decision-neutral-icon" role="img" aria-label={provider === 'typesafe' ? 'TypeSafe' : language === 'zh' ? '自定义接入' : 'Custom connection'}><UiIcon name={provider === 'typesafe' ? 'lock' : 'plug'} /></span>;
}
