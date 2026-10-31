import stock from '../../../../../crates/domain/src/routing/fixtures/system_one_c_v1.json' with { type: 'json' };
import guidance from '../../../../../assets/decision-services/default-assessment-guidance.json' with { type: 'json' };
import { defaultJudgment, type Judgment } from './types.ts';

const wrapper = stock.q1.instructions.split(' Distinguish those meanings')[0];

// Missing intent is supported released input. Explicit custom intent always wins.
export function assessmentStandard(judgment: Judgment): 'default' | 'custom' {
  return judgment.competence.standard ?? (judgment.competence.instructions === defaultJudgment.competence.instructions && judgment.competence.criteria.length === 3 && judgment.competence.criteria.every((c, i) => c === defaultJudgment.competence.criteria[i]) ? 'default' : 'custom');
}
export function stockAssessment(judgment: Judgment, native = false): Judgment {
  const result = structuredClone(judgment);
  result.competence.standard = 'default';
  result.competence.instructions = native ? `${stock.q1.instructions.slice(wrapper.length + 1)} ${guidance.feedback}` : defaultJudgment.competence.instructions;
  result.competence.criteria = structuredClone(native ? stock.q1.criteria : defaultJudgment.competence.criteria) as [string, string, string];
  if (native) result.competence.criteria[2] = guidance.highest_criterion;
  return result;
}
export function resolveAssessment(judgment: Judgment, native = false): Judgment {
  if (assessmentStandard(judgment) === 'default') return stockAssessment(judgment, native);
  const result = structuredClone(judgment);
  if (native || result.competence.standard) result.competence.standard = 'custom';
  return result;
}
