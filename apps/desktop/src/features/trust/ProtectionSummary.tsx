import type { Editor, Options } from '../../plan-editor';
import { protectionExplanation } from './protection-copy';
import { isCustomExtension } from '../decision-services/types';

/** Derived from current editor content, so editing a draft cannot leave stale promises. */
export function ProtectionSummary({ editor, options, language }: { editor: Editor; options: Options | null; language: 'zh' | 'en' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const routing = editor.mode === 'custom_branches' ? editor.branch_routing : null;
  if (!routing) return null;
  const custom = isCustomExtension(routing.classifier);
  const names = (bindings: { binding_id: string }[]) => bindings.map(b => options?.candidates.find(c => c.binding_id === b.binding_id)?.display_name ?? text('已选主力', 'Selected Primary')).join(' → ');
  return <section className="editor-section division-protection-summary"><h3>{custom ? text('当前分组选择', 'Current group selection') : text('当前保护机制', 'Current protection')}</h3>
    {routing.branches.map(branch => <p key={branch.id}><strong>{branch.name}</strong> · {branch.primary_candidates.length && !editor.delegation_enabled ? protectionExplanation(routing.classifier, language, names(branch.primary_candidates)) : custom ? text('此分类仅有一个组；可选评分仅用于观察，不会触发自动升级。', 'This category has only one group. Optional scores are observation only and do not trigger an automatic upgrade.') : text('低分只记录，没有主力组可以自动升级。', 'Low scores are recorded; there is no Primary group for automatic upgrading.')}</p>)}
    {editor.delegation_enabled && <p className="field-help">{text('Worker 只使用本路由的模型与所选执行 Agent。父 Agent 模型和其他执行 Agent 不会补救低分；任务完成不等于胜任，没有评分就仍是未评分。', 'The Worker uses this route’s models and selected execution agent. Parent models and other agents do not rescue low scores. Task completion is not competence; no score still means unscored.')}</p>}
  </section>;
}
