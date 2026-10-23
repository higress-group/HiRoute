import type { AvailableModel, ModelFacts, ModelOrigin } from './types';

type Language = 'zh' | 'en';
const publishers: Record<string, { zh: string; en: string }> = {
  anthropic: { zh: 'Anthropic', en: 'Anthropic' },
  openai: { zh: 'OpenAI', en: 'OpenAI' },
  deepseek: { zh: 'DeepSeek', en: 'DeepSeek' },
  moonshot: { zh: '月之暗面', en: 'Moonshot' },
  zhipu: { zh: '智谱', en: 'Zhipu' },
  zai: { zh: 'Z.ai', en: 'Z.ai' },
  qwen: { zh: '通义千问', en: 'Qwen' },
  'alibaba-qwen': { zh: '通义千问', en: 'Qwen' },
  google: { zh: 'Google', en: 'Google' },
  mistral: { zh: 'Mistral', en: 'Mistral' },
  cohere: { zh: 'Cohere', en: 'Cohere' },
  nvidia: { zh: 'NVIDIA', en: 'NVIDIA' },
  liquid: { zh: 'Liquid AI', en: 'Liquid AI' },
  xai: { zh: 'xAI', en: 'xAI' },
};

/** Catalog publisher IDs are stable identifiers; unknown ones fall back to a readable form. */
export function publisherName(id: string, language: Language) {
  const key = id.replace(/^publisher\./, '');
  return publishers[key]?.[language] ?? key.split(/[-_.]/).filter(Boolean).map(part => part[0].toUpperCase() + part.slice(1)).join(' ');
}

export function contextLabel(tokens: number, language: Language) {
  // Providers state windows in binary (262144 = 256K) or decimal (400000 = 400K) units.
  const binary = tokens % 1000 !== 0 && tokens % 1024 === 0;
  const size = tokens >= 1_000_000 ? `${+(tokens / (binary ? 1_048_576 : 1_000_000)).toFixed(1)}M` : `${Math.round(tokens / (binary ? 1024 : 1000))}K`;
  return language === 'zh' ? `${size} 上下文` : `${size} context`;
}

export function factTags(facts: ModelFacts | null, language: Language) {
  if (!facts) return [];
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const tags = [contextLabel(facts.context_tokens, language)];
  if (facts.tool) tags.push(t('工具调用', 'Tool calls'));
  if (facts.vision) tags.push(t('图片输入', 'Image input'));
  const reasoning = facts.reasoning;
  if (reasoning?.kind === 'toggle') tags.push(t('思考 开 / 关', 'Thinking on / off'));
  if (reasoning?.kind === 'budget') tags.push(t('思考预算', 'Thinking budget'));
  if (reasoning?.kind === 'profiles' && reasoning.profiles.length) tags.push(`${t('思考', 'Thinking')} ${reasoning.profiles.join(' / ')}`);
  return tags;
}

export function shortDate(day: string, language: Language) {
  const date = new Date(`${day}T00:00:00Z`);
  if (!Number.isFinite(date.valueOf())) return day;
  return date.toLocaleDateString(language === 'zh' ? 'zh-CN' : 'en-US', { month: language === 'zh' ? 'numeric' : 'short', day: 'numeric', timeZone: 'UTC' });
}

/** Origin without iteration traces: the feed inclusion date, or the catalog built into this app. */
export function originLabel(origin: ModelOrigin, language: Language) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (origin.kind === 'feed') return t(`hiroute.ai 资料日期 ${shortDate(origin.as_of, language)}`, `hiroute.ai information as of ${shortDate(origin.as_of, language)}`);
  if (origin.kind === 'embedded') return t('HiRoute 内置', 'Built into HiRoute');
  return t('仅在已发布路由中', 'Only in published routes');
}

export const rollingIdentity = (stability: string | null) => ['rolling-alias', 'dynamic-route', 'unknown'].includes(stability ?? 'unknown');

export function modelSubtitle(model: AvailableModel, language: Language) {
  return [model.facts ? publisherName(model.facts.publisher_id, language) : null, originLabel(model.origin, language)].filter(Boolean).join(' · ');
}
