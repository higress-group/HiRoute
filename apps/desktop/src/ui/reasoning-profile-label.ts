/** Display common native profiles without changing their authored wire values. */
export function reasoningProfileLabel(profile: string, language: 'zh' | 'en'): string {
  if (language !== 'zh') return profile;
  const labels: Record<string, string> = {
    none: '不使用思考', minimal: '极低', low: '低', medium: '中',
    high: '高', xhigh: '很高', max: '最高', 'provider-default': '模型默认',
  };
  return labels[profile] ?? profile;
}
