import { ProviderIcon } from '../../ui/ProviderIcon';
import { UiIcon } from '../../ui';

const providerNames = new Map([['openai', 'OpenAI'], ['openrouter', 'OpenRouter'], ['typesafe', 'TypeSafe']]);

export function DecisionIcon({ provider, language }: { provider?: string; language: 'zh' | 'en' }) {
  const providerId = provider?.startsWith('bailian') ? 'bailian' : provider;
  const label = providerId === 'bailian' ? (language === 'zh' ? '百炼' : 'Bailian') : providerNames.get(providerId ?? '');
  if (label) return <ProviderIcon providerId={providerId} label={label} language={language} />;
  return <span className="model-avatar decision-neutral-icon" role="img" aria-label={language === 'zh' ? '自定义接入' : 'Custom connection'}><UiIcon name="plug" /></span>;
}
