export function selectionReasonLabel(reason: string, language: 'zh' | 'en'): string {
  const labels: Record<string, [string, string]> = {
    extension_decision: ['由决策扩展选择', 'Selected by the decision extension'],
    decision_unavailable: ['扩展未提供有效模型组', 'Extension group unavailable'],
    simple_task: ['简单任务', 'Simple task'],
    complex_task: ['复杂任务', 'Complex task'],
    low_competence: ['低分保护', 'Low-score protection'],
    degree_unavailable: ['难度判断不可用', 'Difficulty unavailable'],
    single_group: ['只有一组', 'Single group'],
    decision_fallback: ['判断失败兜底', 'Judgment fallback'],
    availability_relay: ['故障接力', 'Availability relay'],
    heuristic: ['内置规则', 'Built-in rules'],
  };
  return (labels[reason] ?? ['选择原因未记录', 'Selection reason unavailable'])[language === 'zh' ? 0 : 1];
}
