import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { type Plan, type Options } from './plan-editor';

/** Read-only projection; saving still rechecks the exact publication and native configuration. */
export function AgentCapabilityPreview({ agent, plans, language }: { agent: 'codex' | 'claude'; plans: Plan[]; language: 'zh' | 'en' }) {
  const [result, setResult] = useState<Options[] | null>(null);
  const [failed, setFailed] = useState(false);
  const fingerprint = JSON.stringify(plans);
  useEffect(() => {
    let current = true;
    setResult(null); setFailed(false);
    const selected = JSON.parse(fingerprint) as Plan[];
    void Promise.all(selected.map(plan => invoke<Options>('plan_editor_options', { input: { published_plan: { plan_id: plan.agent_plan_id, revision: plan.agent_plan_revision } } }))).then(value => { if (current) setResult(value); }).catch(() => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [fingerprint]);
  if (!plans.length) return null;
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const windows = result?.map(option => agent === 'codex' ? option.codex_capabilities?.state === 'available' ? option.codex_capabilities.context_window : null : option.claude_capabilities?.state === 'available' ? option.claude_capabilities.context_window : null);
  return <div className="option-panel" data-agent-capability-preview={agent} aria-live="polite">
    <strong>{agent === 'codex' ? 'Codex' : 'Claude Code'} · {text('能力预览', 'Capability preview')}</strong>
    {failed ? <p>{text('暂时无法获取能力预览；请重新打开配置重试。', 'Capability preview is unavailable. Reopen settings to retry.')}</p> : !result ? <p>{text('正在核对所选计划…', 'Checking selected plans…')}</p> : <>
      {plans.map((plan, index) => <p key={plan.agent_plan_id}>{plan.desired.display_name} · {windows?.[index] != null ? `${windows[index]!.toLocaleString()} tokens` : text('当前不可用，请检查计划的入口能力和窗口', 'Unavailable; check plan ingress capabilities and window')}</p>)}
      {agent === 'claude' && windows?.every(value => value != null) && <p>{text('进程共同自动压缩窗口', 'Shared process auto-compact window')} · {Math.min(...windows as number[]).toLocaleString()} tokens</p>}
    </>}
    <p className="field-help">{agent === 'claude' ? text('三个预设共享最小窗口（100K–1M）；保留的原生模型可能更早压缩。', 'The three presets share the smallest window (100K–1M); preserved native models may compact earlier.') : text('各计划分别使用自己的窗口。', 'Each plan uses its own window.')}{text('保存时重新核对；重新启动客户端后加载，当前会话不会立即改变。', 'Rechecked on save and loaded after restarting the client; current sessions do not change immediately.')}</p>
  </div>;
}
