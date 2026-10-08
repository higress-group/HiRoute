export type QualityExecution = {
  group: 'regular' | 'primary' | null;
  candidate_index: number | null;
  plan_revision: number;
  selected_branch_id: string | null;
  executed_branch_id: string | null;
  model_configuration_id: string | null;
  profile_digest: string | null;
  attribution: 'single' | 'mixed' | 'unknown';
};

export type BranchPolicy = { name: string; floor_millis: number; criteria_digest?: string | null };

export type QualityModelSummary = {
  branch_policy?: BranchPolicy | null;
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
  plan_revision: number;
  branch_name?: string;
  group: 'regular' | 'primary';
  candidate_index: number;
  floor_millis?: number;
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
    execution.model_configuration_id, execution.profile_digest, execution.attribution, execution.group, execution.candidate_index,
  ]);
}

/** Preserve published candidate order; never merge distinct execution profiles
 * or sort models by score. Historical/unattributable rows retain their own key.
 * A source revision changes its materialized model ID. Join the published slot,
 * not today's editor-option ID; keep the observed identity for statistics/drill-down.
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
      && s.execution.plan_revision === model.plan_revision
      && s.execution.executed_branch_id === model.branch_id
      && s.execution.group === model.group && s.execution.candidate_index === model.candidate_index
      && (s.reasoning_profile_id === model.reasoning_profile_id
        || (!s.reasoning_profile_id && configured.filter(c => c.plan_revision === model.plan_revision && c.branch_id === model.branch_id
          && c.group === model.group && c.candidate_index === model.candidate_index).length === 1)));
    if (!matches.length) rows.push({
      key: JSON.stringify(['configured', model.plan_revision, model.branch_id, model.model_configuration_id, model.reasoning_profile_id, model.group, model.candidate_index]),
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
    smart_saving: ['智能省钱', 'Smart saving'],
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
  if (profile && /^budget-[0-9]+$/.test(profile)) return profile.slice(7) + ' tokens';
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

export function qualityGroupLabel(branch: string, group: 'regular' | 'primary' | null | undefined, language: 'zh' | 'en'): string {
  if (!group) return language === 'zh' ? '模型组未记录' : 'Model group unrecorded';
  return group === 'primary' ? (language === 'zh' ? '主力' : 'Primary')
    : branch === 'smart_saving' ? (language === 'zh' ? '省钱' : 'Economy') : (language === 'zh' ? '常规' : 'Regular');
}
export function qualityRowSection(row: QualityModelRow): string {
  return JSON.stringify([row.branch, row.summary?.execution.group ?? row.configured?.group ?? null]);
}
export function qualityRowSectionLabel(row: QualityModelRow, language: 'zh' | 'en'): string {
  const group = qualityGroupLabel(row.branch, row.summary?.execution.group ?? row.configured?.group, language);
  if (row.branch === 'smart_saving') return group;
  const name = row.configured?.branch_name ?? row.summary?.branch_policy?.name ?? qualityBranchLabel(row.branch, language);
  return `${name} → ${group}`;
}
