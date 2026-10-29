import type { Classifier } from '../decision-services/types';

/** Authoring uses the same native-mode condition as the backend's next publication. */
export function protectionExplanation(classifier: Classifier, language: 'zh' | 'en', primary = language === 'zh' ? '主力模型' : 'Primary'): string {
  if (classifier.kind === 'decision_service' && classifier.service.connection.kind === 'custom') return language === 'zh'
    ? '自定义扩展按自己的策略选择常规或主力组；可选评分仅用于观察。HiRoute 不应用阈值或低分保护。'
    : 'The extension chooses Regular or Primary using its own policy. Optional scores are observation only; HiRoute applies no threshold or low-score protection.';
  const native = classifier.kind === 'decision_service' && classifier.service.connection.kind === 'system_one';
  if (native) return language === 'zh'
    ? `上一实际阶段的完整评分低于胜任下限时，若之前的问题仍相关，本轮选择${primary}；无法判断时保守地保留低分保护。问题已修正或无关时，本轮按难度选择模型。低分保护适用时，即使上一阶段已使用主力，本轮仍选择主力，不会出现第三组。`
    : `After an eligible complete score below the competence floor for the previous actual stage, this turn selects ${primary} if the earlier problem still matters. If that cannot be checked, low-score protection stays in place. If it is fixed or unrelated, this turn selects a model based on difficulty. When low-score protection applies, a previous Primary stage still selects Primary for this turn; there is no third group.`;
  return language === 'zh'
    ? '本地规则按当前任务信号选择模型，不评估上一阶段，也不应用低分保护。'
    : 'Local rules select a model from current task signals; they do not assess the previous stage or apply low-score protection.';
}
