import table from '../../../../../contracts/routing/trust-levels.v1.json' with { type: 'json' };
export const trustLevels = table.levels;
export function trustLevelLabel(threshold: number, language: 'zh' | 'en'): string {
  const level = trustLevels.find(item => item.threshold_millis === threshold);
  return level ? level.label[language] : `${language === 'zh' ? '自定义' : 'Custom'} ${threshold / 1000}`;
}
