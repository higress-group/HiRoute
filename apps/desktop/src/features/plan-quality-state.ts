export type QualityExecution = {
  plan_revision: number;
  selected_branch_id: string | null;
  executed_branch_id: string | null;
  model_configuration_id: string | null;
  profile_digest: string | null;
  attribution: 'single' | 'mixed' | 'unknown';
};

export type QualityModelSummary = {
  execution: QualityExecution;
  native_model: string | null;
  reasoning_profile_id: string | null;
  scored_stage_count: number;
  unrated_stage_count: number;
  average_score: number | null;
};

export type QualitySummary = {
  models: QualityModelSummary[];
  scored_stage_count: number;
  unrated_stage_count: number;
  session_count: number;
  available_revisions: number[];
};

export type PlanQualityModel = {
  model_configuration_id: string;
  display_name: string;
  branch_id: string;
  reasoning_profile_id: string | null;
};

export type QualityModelRow = {
  key: string;
  branch: string;
  configured?: PlanQualityModel;
  summary?: QualityModelSummary;
};

export function qualityExecutionKey(execution: QualityExecution): string {
  return JSON.stringify([
    execution.plan_revision, execution.selected_branch_id, execution.executed_branch_id,
    execution.model_configuration_id, execution.profile_digest, execution.attribution,
  ]);
}

/** Preserve published candidate order; never merge distinct execution profiles
 * or sort models by score. Historical/unattributable rows retain their own key.
 */
export function qualityModelRows(configured: readonly PlanQualityModel[], summaries: readonly QualityModelSummary[]): QualityModelRow[] {
  const remaining = new Set(summaries);
  const rows: QualityModelRow[] = [];
  const branchOf = (s: QualityModelSummary) => s.execution.attribution === 'single'
    ? s.execution.executed_branch_id ?? 'unrecorded'
    : 'unattributed';
  for (const model of configured) {
    const matches = summaries.filter(s => remaining.has(s)
      && s.execution.attribution === 'single'
      && s.execution.executed_branch_id === model.branch_id
      && s.execution.model_configuration_id === model.model_configuration_id
      && (s.reasoning_profile_id === model.reasoning_profile_id
        || (!s.reasoning_profile_id && configured.filter(c => c.branch_id === model.branch_id
          && c.model_configuration_id === model.model_configuration_id).length === 1)));
    if (!matches.length) rows.push({
      key: JSON.stringify(['configured', model.branch_id, model.model_configuration_id, model.reasoning_profile_id]),
      branch: model.branch_id, configured: model,
    });
    for (const summary of matches) {
      remaining.delete(summary);
      rows.push({ key: qualityExecutionKey(summary.execution), branch: branchOf(summary), configured: model, summary });
    }
  }
  for (const summary of summaries) {
    if (remaining.has(summary)) rows.push({ key: qualityExecutionKey(summary.execution), branch: branchOf(summary), summary });
  }
  return rows;
}

export function qualityBranchLabel(branch: string, language: 'zh' | 'en'): string {
  const labels: Record<string, [string, string]> = {
    smart_saving_simple: ['日常执行 · 省钱分组', 'Routine work · Economy group'],
    smart_saving_complex: ['复杂推理 · 主力分组', 'Complex work · Primary group'],
    fixed: ['固定候选', 'Fixed candidates'],
    fixed_model: ['固定候选', 'Fixed candidates'],
    free: ['免费候选', 'Free candidates'],
    free_first: ['免费候选', 'Free candidates'],
    primary: ['主力分组', 'Primary group'],
    unrecorded: ['执行分组未记录', 'Execution group unrecorded'],
    unattributed: ['执行身份未确认', 'Unattributed execution'],
  };
  return labels[branch]?.[language === 'zh' ? 0 : 1] ?? (language === 'zh' ? '其他执行分组' : 'Other execution group');
}

export function qualityReasoningLabel(profile: string | null | undefined, language: 'zh' | 'en'): string {
  const labels: Record<string, [string, string]> = {
    fixed: ['固定思考设置', 'Fixed reasoning'],
    default: ['默认思考设置', 'Default reasoning'],
    none: ['关闭思考', 'Reasoning off'],
    disabled: ['关闭思考', 'Reasoning off'],
    enabled: ['开启思考', 'Reasoning on'],
    minimal: ['最小思考强度', 'Minimal reasoning'],
    low: ['低思考强度', 'Low reasoning'],
    medium: ['中等思考强度', 'Medium reasoning'],
    high: ['高思考强度', 'High reasoning'],
    xhigh: ['更高思考强度', 'Extra high reasoning'],
  };
  if (profile && labels[profile]) return labels[profile][language === 'zh' ? 0 : 1];
  if (profile && /^budget-[0-9]+$/.test(profile)) return language === 'zh'
    ? '思考预算 ' + profile.slice(7) + ' tokens' : 'Reasoning budget ' + profile.slice(7) + ' tokens';
  // Native authored profile names are readable; a digest/path is diagnostic only.
  if (profile && profile.length <= 32 && /^[a-zA-Z][a-zA-Z0-9 _-]*$/.test(profile)) return profile;
  return language === 'zh' ? '推理配置未记录' : 'Reasoning configuration unrecorded';
}

/** CPA uses a routing prefix to pin the current Codex account. It is transport
 * identity, not part of the model's human-facing name. Preserve other namespaces.
 */
export function qualityNativeModelName(nativeModel: string): string {
  const prefix = 'hiroute-codex-current/';
  return nativeModel.startsWith(prefix) && nativeModel.length > prefix.length
    ? nativeModel.slice(prefix.length) : nativeModel;
}
