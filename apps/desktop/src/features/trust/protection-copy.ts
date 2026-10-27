import type { Classifier } from '../decision-services/types';

/** Authoring uses the same native-mode condition as the backend's next publication. */
export function protectionExplanation(classifier: Classifier, language: 'zh' | 'en', primary = language === 'zh' ? '主力模型' : 'Primary'): string {
  if (classifier.kind === 'decision_service' && classifier.service.connection.kind === 'custom') return language === 'zh'
    ? '自定义扩展按自己的策略选择常规或主力组；可选评分仅用于观察。HiRoute 不应用阈值或低分保护。'
    : 'The extension chooses Regular or Primary using its own policy. Optional scores are observation only; HiRoute applies no threshold or low-score protection.';
  const native = classifier.kind === 'decision_service' && classifier.service.connection.kind === 'system_one';
  if (native) return language === 'zh'
    ? `常规模型上一阶段的完整评分低于胜任下限时，若之前的问题仍相关，下一轮交给${primary}；无法判断时保守地保留保护。问题已修正或无关时，简单任务仍可交给常规模型。`
    : `After a complete Regular score below the competence floor, the next turn goes to ${primary} if the earlier problem still matters. If that cannot be checked, protection stays in place. If it is fixed or unrelated, simple tasks can stay with Regular.`;
  return language === 'zh'
    ? `常规模型上一阶段的完整评分低于胜任下限时，下一轮交给${primary}。`
    : `When Regular's previous stage has a complete score below the competence floor, the next agent turn goes to ${primary}.`;
}
