import { useEffect, useMemo, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { Editor, Options, Plan } from '../../plan-editor';
import { Dialog } from '../../ui';
import { divisionDraft, divisionSource } from './division-draft';
import { protectionExplanation } from './protection-copy';
import { isCustomExtension } from '../decision-services/types';

type Harness = NonNullable<Editor['work']>['harness'];
const harnessNames: Record<Harness, string> = { codex_cli: 'Codex CLI', claude_code: 'Claude Code', deepseek_harness: 'DeepSeek Harness', pi: 'Pi', qoder_cli: 'Qoder CLI', opencode: 'OpenCode' };
export function DivisionDraftDialog({ plan, editor, branch, language, options, onClose, onDraft }: { plan: Plan; editor: Editor; branch: string; language: 'zh' | 'en'; options: Options | null; onClose(): void; onDraft(editor: Editor): Promise<boolean> }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const source = useMemo(() => divisionSource(editor, branch), [editor, branch]);
  const custom = source && isCustomExtension(source.classifier);
  const [kind, setKind] = useState<'branches' | 'worker'>('branches');
  const [keepPrimary, setKeepPrimary] = useState(false);
  const [harness, setHarness] = useState<Harness | null>(null);
  const [availability, setAvailability] = useState<{ harness: Harness; state: string }[] | null>(null);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [busy, setBusy] = useState(false);
  const [design, setDesign] = useState(text('理解需求，比较方案，设计架构和拆分实施步骤。', 'Understand requirements, compare approaches, design architecture and plan implementation steps.'));
  const [implementation, setImplementation] = useState(text('按照已确认方案实现、调试、测试并完成有明确范围的修改。', 'Implement, debug and test bounded changes following an agreed design.'));
  useEffect(() => {
    if (kind !== 'worker') return;
    let alive = true; setError('');
    invoke<{ executors: { harness: Harness; state: string }[] }>('worker_executor_availability').then(value => {
      if (!Array.isArray(value.executors)) throw Error('invalid response');
      if (alive) setAvailability(value.executors);
    }).catch(() => { if (alive) setError(text('暂时无法检查执行 Agent。', 'Execution agents could not be checked.')); });
    return () => { alive = false; };
  }, [kind, retry]);
  const name = (bindings: { binding_id: string }[]) => bindings.map(s => options?.candidates.find(c => c.binding_id === s.binding_id)?.display_name ?? text('已选模型', 'Selected model')).join(' → ');
  const trial = source ? name(source.regular) : '';
  const primary = source ? name(source.primary) : '';
  const deepseek = source?.regular.some(s => options?.candidates.some(c => c.binding_id === s.binding_id && /deepseek/i.test(`${c.model_configuration_id} ${c.display_name}`)));
  const executors = (Object.keys(harnessNames) as Harness[]).sort((a, b) => deepseek ? Number(b === 'deepseek_harness') - Number(a === 'deepseek_harness') : 0);
  async function create() {
    if (!source || busy || (kind === 'worker' && !harness)) return;
    setBusy(true);
    try {
      const fresh = await invoke<Options>('plan_editor_options', { input: { authoring: {} } });
      if (!fresh.authoring) throw Error('SCHEMA_INCOMPATIBLE');
      const draft = divisionDraft(plan, source, kind, keepPrimary, [design, implementation], [text('设计与规划', 'Design and planning'), text('实现', 'Implementation')], harness ? { harness, protocol: harness === 'claude_code' ? 'messages' : 'responses' } : undefined, fresh.authoring.editor);
      if (await onDraft(draft)) onClose();
    } catch { setError(text('草稿暂时无法打开，请重试。', 'The draft could not be opened. Try again.')); } finally { setBusy(false); }
  }
  return <Dialog open title={text('探索分工', 'Explore division')} closeLabel={text('关闭分工草稿', 'Close division draft')} closeDisabled={busy} onClose={onClose} footer={<><button className="btn" disabled={busy} onClick={onClose}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" disabled={busy || !source || !design.trim() || !implementation.trim() || (kind === 'worker' && !harness)} onClick={() => void create()}>{text('在编辑器中查看草稿', 'Review draft in editor')}</button></>}>
    {!source ? <p role="alert">{text('此分类目前没有可用于分工的常规和主力组。', 'This category has no Regular and Primary groups available for division.')}</p> : <div className="division-draft-form">
      <div className="segmented"><button className={`segment${kind === 'branches' ? ' active' : ''}`} aria-pressed={kind === 'branches'} onClick={() => setKind('branches')}>{text('模型按任务分工', 'Divide by task')}</button><button className={`segment${kind === 'worker' ? ' active' : ''}`} aria-pressed={kind === 'worker'} onClick={() => setKind('worker')}>{text('委派给执行 Agent', 'Delegate to an agent')}</button></div>
      <label className="field"><span className="field-label">{text('设计与规划的条件', 'Design and planning condition')}</span><textarea className="input" rows={3} value={design} onChange={e => setDesign(e.target.value)} /></label>
      <label className="field"><span className="field-label">{text('实现的条件', 'Implementation condition')}</span><textarea className="input" rows={3} value={implementation} onChange={e => setImplementation(e.target.value)} /></label>
      {kind === 'branches' ? <label className="check-row"><input type="checkbox" checked={keepPrimary} onChange={e => setKeepPrimary(e.target.checked)} /><span>{custom ? text(`实现分类保留 ${primary} 主力组，供扩展选择`, `Keep ${primary} as a Primary group the extension can choose for Implementation`) : text(`实现分类保留 ${primary} 作为低胜任度保护的主力`, `Keep ${primary} as Primary for low-competence protection in Implementation`)}</span></label> : <div className="division-executors"><strong>{text('选择执行 Agent', 'Choose an execution agent')}</strong>{error && <div className="callout warn" role="alert">{error}<button className="btn" onClick={() => setRetry(n => n + 1)}>{text('重试', 'Retry')}</button></div>}{executors.map(id => { const state = availability?.find(e => e.harness === id)?.state; return <label className="check-row" key={id}><input type="radio" name="division-harness" checked={harness === id} onChange={() => setHarness(id)} /><span>{harnessNames[id]} · {state === 'ready' ? text('依赖已就绪', 'Dependencies ready') : state === 'unavailable' ? text('依赖未就绪', 'Dependencies unavailable') : text('依赖尚未确认', 'Dependencies unverified')}</span></label>; })}<p className="field-help">{text('草稿不会安装或启动 Worker。发布前仍需完成依赖检查。', 'The draft does not install or start a Worker. Dependency checks still apply before publication.')}</p></div>}
      <div className="callout warn division-protection"><div><strong>{custom ? text('分组选择的变化', 'Group selection changes') : text('保护机制的变化', 'Protection changes')}</strong>
        {kind === 'branches' ? <><p>{custom ? text(`设计与规划只用 ${primary}；可选评分仅用于观察。`, `Design and planning uses only ${primary}. Optional scores are observation only.`) : text(`设计与规划只用 ${primary}；低分只记录，没有主力组可升级。`, `Design and planning uses ${primary}. Low scores are recorded; there is no Primary group to upgrade to.`)}</p><p>{keepPrimary ? <>{text(`实现使用 ${trial}。`, `Implementation uses ${trial}. `)}{protectionExplanation(source.classifier, language, primary)}</> : custom ? text(`实现只用 ${trial}；可选评分仅用于观察，不会触发自动升级。`, `Implementation uses only ${trial}. Optional scores are observation only and do not trigger an automatic upgrade.`) : text(`实现只用 ${trial}；低分只记录，没有主力组可升级。`, `Implementation uses ${trial}. Low scores are recorded; there is no Primary group to upgrade to.`)}</p></> : <p>{custom ? text(`Worker 的两个分类都只用 ${trial}${harness ? ' · ' + harnessNames[harness] : ''}。可选评分仅用于观察，不会触发自动升级；父 Agent 的模型和其他执行 Agent 不会补救低分。任务完成不等于胜任。`, `Both Worker categories use only ${trial}${harness ? ' in ' + harnessNames[harness] : ''}. Optional scores are observation only and do not trigger an automatic upgrade; parent models and other agents do not rescue a low score. Completion is not competence.`) : text(`Worker 的两个分类都只用 ${trial}${harness ? ' · ' + harnessNames[harness] : ''}。低分只记录，父 Agent 的模型和其他执行 Agent 不会补救低分。任务完成不等于胜任。`, `Both Worker categories use only ${trial}${harness ? ' in ' + harnessNames[harness] : ''}. Low scores are recorded; parent models and other agents do not rescue a low score. Completion is not competence.`)}</p>}
      </div></div>
      <p className="field-help">{custom ? text(`现有证据只覆盖扩展实际交给 ${trial} 的阶段，不能证明它能胜任此前仅由主力处理的工作。`, `Existing evidence covers only stages the extension actually routed to ${trial}; it does not establish competence on work handled only by Primary.`) : text(`现有证据来自阈值 ${(source.threshold / 1000).toFixed(2)} 下交给 ${trial} 的阶段。此前由主力处理的工作尚未验证。`, `Evidence comes from stages routed to ${trial} at threshold ${(source.threshold / 1000).toFixed(2)}. Work handled by Primary is untested.`)}</p>
      <p className="field-help">{text('将打开独立草稿；保存和发布仍由你确认，Agent 映射不会自动改变。', 'This opens a separate draft. You still confirm saving and publication; Agent mappings do not change automatically.')}</p>
    </div>}
  </Dialog>;
}
