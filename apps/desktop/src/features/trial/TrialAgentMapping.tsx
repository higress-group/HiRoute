import { TrialSessionHandoff } from './TrialSessionHandoff';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { Outcome, Preview } from '../agents/types';
import { trialModelChoices, type TrialAgentChoices, type TrialAgentSnapshot } from './agent-read';
import { useTrialAgents } from './useTrialAgents';
import { agentDisplayName, agentEcosystem } from '../agents/ecosystems';
import { agentEditorSeed, editorFingerprint } from '../agents/editor-state';
import { agentModelStatus } from '../agents/status';
import { agentSettingsSpec, prerequisiteCheck } from '../agents/settings-request';
import { classifyAgentMutation } from '../agents/mutation-feedback';
import { AgentSettingsFeedback } from '../agents/AgentSettingsFeedback';
import type { OperationReference } from '../model-connections/types';
import type { Plan } from '../../plan-editor';
import { initialTrialMapping, trialMappingValues, type TrialMapping } from './model';

export function TrialAgentChoice({ snapshot, value, onChange, language, disabled = false, loading = false, onRetry }: {
  snapshot: TrialAgentSnapshot; value: TrialMapping; onChange(value: TrialMapping): void; language: 'zh' | 'en'; disabled?: boolean; loading?: boolean; onRetry?(): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const usable = trialModelChoices(snapshot.choices);
  const agent = snapshot.selected?.agent_id === value.agentId ? snapshot.selected : null;
  const known = !loading && !!agent && agentEditorSeed(agent, 'model', []).known;
  const selection = agent && agentModelStatus(agent)?.current_selection;
  const current = (preset: 'opus' | 'sonnet' | 'haiku') => {
    const choice = selection?.mode === 'claude_launcher' ? selection.preset_mappings[preset] : null;
    if (choice?.kind === 'plan') return snapshot.plans.plans.find(plan => plan.agent_plan_id === choice.plan_id)?.desired.display_name ?? t('已删除的路由', 'A removed route');
    return t('原生模型', 'Native model');
  };
  return <fieldset disabled={disabled} className="trial-agent-choice">
    {usable.length > 1 ? <label className="field"><span className="field-label">Agent</span><select className="input" value={value.agentId} onChange={e => onChange({ ...value, agentId: e.target.value })}><option value="">{t('选择 Agent', 'Select an Agent')}</option>{usable.map(item => <option key={item.agent_id} value={item.agent_id}>{agentDisplayName(item.agent_id, language)}</option>)}</select></label>
      : !usable.length && <p className="field-help">{t('还没有可以修改模型设置的 Agent。发布后可以到 Agent 页面接入。', 'No Agent with editable model settings yet. You can connect one from Agents after publishing.')}</p>}
    {agentEcosystem(value.agentId) === 'claude' ? <div className="trial-presets" role="group" aria-label={t('应用到哪些预设', 'Presets to change')}>
      <span className="trial-presets-title">{t(`${agentDisplayName(value.agentId, language)} 的模型预设`, `${agentDisplayName(value.agentId, language)} model presets`)}</span>
      {(['opus', 'sonnet', 'haiku'] as const).map(preset => <label key={preset} className="trial-preset"><input type="checkbox" disabled={!known} checked={value.presets.includes(preset)} onChange={e => onChange({ ...value, presets: e.target.checked ? [...value.presets, preset] : value.presets.filter(p => p !== preset) })} /><strong>{preset[0].toUpperCase() + preset.slice(1)}</strong><span>{t('现在：', 'Now: ')}{known ? current(preset) : t('尚未核实', 'Not yet verified')}</span></label>)}
    </div> : value.agentId && <p className="field-help">{agentDisplayName(value.agentId, language)} · {agentEcosystem(value.agentId) === 'codex' ? t('将此路由设为默认模型；保留已有可用路由。', 'Use this route as the default model and retain existing allowed routes.') : t('将此路由加入可用路由。', 'Add this route to the allowed routes.')}</p>}
    {loading ? <p role="status">{t('正在读取所选 Agent 的模型配置…', 'Reading the selected Agent model settings…')}</p>
      : value.agentId && !known && <p role="alert">{t('所选 Agent 的模型配置尚未确认。请重试或到 Agent 页面处理。', 'The selected Agent model settings are unconfirmed. Retry or inspect the Agent page.')}{onRetry && <button type="button" className="btn btn-quiet" onClick={onRetry}>{t('重试', 'Retry')}</button>}</p>}
    <p className="field-help">{t('只调整选中的模型路由。权限、工具审批和任务协作不变。', 'Only the selected model routing changes. Permissions, tool approval and task collaboration stay unchanged.')}</p>
  </fieldset>;
}

/** A published Plan survives every mapping failure and can be mapped here after reload. */
export function TrialAgentMapping({ plan, initial, initialChoices, language, mutable, onOperation, onChanged, onOpenAgent, onOpenPerformance, onOpenSessions, onConfirmed }: {
  initialChoices?: TrialAgentChoices | null;
  onOpenPerformance?(): void; onOpenSessions?(sessionId?: string): void; onConfirmed?(): void;
  plan: Plan; initial?: TrialMapping; language: 'zh' | 'en'; mutable: boolean;
  onOperation(operation: OperationReference): void; onChanged(): void; onOpenAgent?(id: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [mapping, setMapping] = useState<TrialMapping | null>(initial ?? null);
  const agentRead = useTrialAgents(mapping?.agentId ?? '', initialChoices);
  const snapshot = agentRead.snapshot;
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [pending, setPending] = useState<OperationReference | 'unverified' | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [done, setDone] = useState<number | null>(null);
  const requested = useRef<{ context: string; fingerprint: string } | null>(null);
  const inFlight = useRef(false);
  async function refresh(observedPending = pending, discover = false) {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const current = await agentRead.refresh(discover);
      if (!current) return;
      // Explicit recovery may discover a different native target. It never
      // submits an intent or relaxes the exact pending-operation comparison.
      if (discover && !observedPending) { setError(''); setPreview(null); }
      setMapping(value => value ?? initialTrialMapping(current));
      if (observedPending && mapping && requested.current) {
        const agent = current.selected?.agent_id === mapping.agentId ? current.selected : null;
        const status = agent && agentModelStatus(agent);
        const exact = agent?.context_id === requested.current.context && status?.state === 'configured'
          && editorFingerprint(agentEditorSeed(agent, 'model', [])) === requested.current.fingerprint;
        // For an unknown submission result the settings are the authority, but no
        // unrelated global operation is treated as this flow's completion.
        if (exact && (observedPending === 'unverified' || status?.operation_id === observedPending.operation_id || observedPending.state === 'succeeded')) {
          setDone(previous => previous ?? Date.now()); setPending(null); setError(''); onChanged();
        } else if (observedPending !== 'unverified' && status?.operation_id === observedPending.operation_id && ['rolled_back', 'failed', 'cancelled', 'compensated'].includes(status.operation_state ?? '')) {
          setPending(null); setError(t('路由已发布，但 Agent 映射未生效。请检查状态后重试。', 'The route is published, but the Agent mapping did not apply. Review its state before retrying.'));
        }
      }
    } catch { setError(t('暂时无法核实 Agent 配置。请刷新结果。', 'Agent settings could not be verified. Refresh the result.')); }
    finally { inFlight.current = false; }
  }
  useEffect(() => { if (!mapping && snapshot && agentRead.ready) setMapping(initialTrialMapping(snapshot)); }, [snapshot, agentRead.ready, mapping]);
  useEffect(() => { if (done !== null) onConfirmed?.(); }, [done]);
  useEffect(() => {
    if (!pending) return;
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2000);
    return () => window.clearInterval(timer);
  }, [pending, mapping]);
  async function apply() {
    if (busy || pending || !snapshot || !mapping || !mutable) return;
    setBusy(true); setError(''); setPreview(null);
    let submitted = false;
    try {
      const reviewed = snapshot.selected;
      const current = await agentRead.refresh();
      if (!current) throw Error('AGENT_STATUS_UNAVAILABLE');
      const agent = current.selected?.agent_id === mapping.agentId ? current.selected : null;
      if (!reviewed || !agent || !current.trusted_authority || reviewed.context_id !== agent.context_id
        || editorFingerprint(agentEditorSeed(reviewed, 'model', [])) !== editorFingerprint(agentEditorSeed(agent, 'model', []))) {
        setError(t('Agent 配置已变化。已读入最新配置，请重新核对并确认。', 'Agent settings changed. Review the refreshed configuration and confirm again.')); return;
      }
      const values = trialMappingValues(agent, plan.agent_plan_id, mapping);
      const spec = agentSettingsSpec(agent, 'model', false, { values, codexMode: agent.codex_access?.selected_mode ?? 'profile', restoreNativeModel: '' });
      requested.current = { context: spec.context_id, fingerprint: editorFingerprint(values) };
      const input = { language, spec };
      submitted = true;
      let result = await invoke<Outcome>('preview_agent_settings', { input });
      submitted = false;
      const check = prerequisiteCheck(result.preview, 'model');
      if (check) {
        setPreview(result.preview);
        const accepted = await invoke<boolean>('check_agent_authentication', { input: { agent_id: agent.agent_id, language, scope: check } });
        if (!accepted) return;
        submitted = true; result = await invoke<Outcome>('preview_agent_settings', { input }); submitted = false;
      }
      setPreview(result.preview);
      if (result.mutation && classifyAgentMutation(result.mutation) !== 'cancelled') {
        setPending(result.mutation.operation ?? 'unverified');
        if (result.mutation.operation) onOperation(result.mutation.operation);
        // Read the exact submitted outcome immediately. The periodic read is only
        // recovery for an operation still in flight, not a minimum completion delay.
        await refresh(result.mutation.operation ?? 'unverified');
      }
    } catch {
      if (submitted) setPending('unverified');
      setError(t('映射尚未确认。已发布路由会保留，请刷新结果或到 Agent 页面处理。', 'Mapping is unconfirmed. The published route is retained; refresh the result or inspect the Agent page.'));
    } finally { setBusy(false); }
  }
  return <section className="trial-mapping"><h3>{done ? t('路由已发布，Agent 已映射', 'Route published and Agent mapped') : t('路由已发布，接下来映射 Agent', 'Route published. Map an Agent next.')}</h3><p className="trial-mapping-note">{plan.desired.display_name}{!done && <> · {t('即使离开，也可从智能路由页面继续映射。', 'You can resume mapping from Routing after leaving this flow.')}</>}</p>
    {(error || (agentRead.error && !snapshot?.choices.length)) && <p role="alert" className="callout warn">{error || t('Agent 配置暂时无法读取，请刷新结果。', 'Agent settings could not be read. Refresh the result.')}</p>}
    {done ? <p role="status" className="callout good"><span>{mapping && <strong>{agentDisplayName(mapping.agentId, language)}{agentEcosystem(mapping.agentId) === 'claude' ? ` · ${mapping.presets.join(', ')}` : ''} — </strong>}{t('映射已确认。回到 Agent 发起新会话，在会话路由时间线中观察试用结果。', 'Mapping confirmed. Start a new session in your Agent and inspect the routing timeline for trial results.')}</span></p> : snapshot && <TrialAgentChoice snapshot={snapshot} value={mapping ?? initialTrialMapping(snapshot)} onChange={setMapping} language={language} disabled={busy || !!pending} loading={agentRead.loading} onRetry={() => void refresh(pending, true)} />}
    {preview && <AgentSettingsFeedback preview={preview} language={language} facet="model" defaultChoice={{ kind: 'plan', plan_id: plan.agent_plan_id }} disabled={busy || !!pending} onCheck={() => void apply()} />}
    {pending && <p role="status">{t('正在核实本次映射；刷新不会重复提交。', 'Verifying this mapping; refresh does not resubmit.')}</p>}
    {done !== null && <TrialSessionHandoff planId={plan.agent_plan_id} confirmedAt={done} language={language} onWatch={onOpenSessions} />}
    <div className="actions">{done && onOpenPerformance && <button className="btn" onClick={onOpenPerformance}>{t('查看模型表现', 'View model performance')}</button>}{done && onOpenSessions && <button className="btn" onClick={() => onOpenSessions()}>{t('查看会话时间线', 'View session timelines')}</button>}<button className="btn btn-quiet" disabled={busy || agentRead.loading} onClick={() => void refresh(pending, true)}>{t('刷新结果', 'Refresh result')}</button>{!done && <button className="btn btn-primary" disabled={busy || !!pending || !mutable || !agentRead.ready || !snapshot?.selected || !agentEditorSeed(snapshot.selected, 'model', []).known || !snapshot?.trusted_authority || !mapping?.agentId || (agentEcosystem(mapping.agentId) === 'claude' && !mapping.presets.length)} onClick={() => void apply()}>{t('检查并映射 Agent', 'Review and map Agent')}</button>}{mapping?.agentId && onOpenAgent && <button className="btn btn-quiet" onClick={() => onOpenAgent(mapping.agentId)}>{t('查看 Agent 配置', 'Inspect Agent settings')}</button>}</div>
  </section>;
}
