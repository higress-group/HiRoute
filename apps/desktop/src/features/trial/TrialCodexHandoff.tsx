import { useEffect, useState } from 'react';
import { invoke } from '../../host';
import { CodexHistoryNotice, CodexLaunchInstructions } from '../agents/CodexAccessPanel';
import { editorFingerprint, agentEditorSeed } from '../agents/editor-state';
import { agentModelStatus } from '../agents/status';
import type { Agent, AgentSnapshot } from '../agents/types';

export type TrialCodexTarget = { contextId: string; fingerprint: string; nativeTarget: Agent['native_target'] };

/** Read the confirmed native target only; do not rescan unrelated Agents. */
export function TrialCodexHandoff({ target, mutable, language }: { target: TrialCodexTarget; mutable: boolean; language: 'zh' | 'en' }) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [agent, setAgent] = useState<Agent | null>(null);
  const [failed, setFailed] = useState(false), [retry, setRetry] = useState(0);
  useEffect(() => {
    let current = true;
    setAgent(null); setFailed(false);
    if (!mutable) return;
    void invoke<AgentSnapshot>('agent_snapshot', { contextId: target.contextId, nativeTargets: target.nativeTarget ? [target.nativeTarget] : [] }).then(snapshot => {
      const selected = snapshot.agents.find(value => value.context_id === target.contextId);
      if (!snapshot.trusted_authority || !selected || !['agent_codex_default', 'agent_codex_desktop'].includes(selected.agent_id) || selected.status_error
        || agentModelStatus(selected)?.state !== 'configured'
        || editorFingerprint(agentEditorSeed(selected, 'model', [])) !== target.fingerprint) throw Error('AGENT_STATUS_UNAVAILABLE');
      if (selected.agent_id === 'agent_codex_desktop') {
        if (!selected.codex_desktop || !selected.available_surfaces?.includes('codex_desktop')
          || agentModelStatus(selected)?.context_id !== target.contextId) throw Error('AGENT_STATUS_UNAVAILABLE');
      } else {
        const access = selected.codex_access;
        if (!access?.slot_occupied || access.pending_operation || access.access_revoked
          || (access.selected_mode === 'profile' ? access.profile_context_id : access.root_context_id) !== target.contextId
          || (access.selected_mode === 'profile' && !Object.values(access.commands).some(Boolean))) throw Error('AGENT_STATUS_UNAVAILABLE');
      }
      if (current) setAgent(selected);
    }).catch(() => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [target, mutable, retry]);
  const access = mutable ? agent?.codex_access : null;
  const desktop = mutable && agent?.agent_id === 'agent_codex_desktop';
  return <section className="trial-codex-handoff" aria-label={t('开始使用 Codex', 'Start using Codex')}>
    {!access && !desktop ? <p className="field-help" role="status">{failed || !mutable
      ? t('启动方式尚未核实，请恢复连接后重读，或查看 Agent 配置。', 'Launch instructions are unconfirmed. Restore the connection and reload, or inspect Agent settings.')
      : t('正在读取 Codex 启动方式…', 'Reading Codex launch instructions…')}{failed && mutable && <button className="btn btn-quiet" onClick={() => setRetry(value => value + 1)}>{t('重读启动方式', 'Reload launch instructions')}</button>}</p>
      : desktop ? <p className="field-help" data-trial-codex-desktop-start>{t('通过下方“查看 Agent 配置”启动独立 Codex Desktop 窗口，或复制本机启动命令。', 'Use Inspect Agent settings below to start the independent Codex Desktop window or copy its local launch command.')}</p>
      : access && <>
        <p className="field-help">{access.selected_mode === 'profile'
          ? t('按需使用 · 仅 Codex CLI。请用下方命令，在安装 Codex 的设备终端开启新会话。直接运行普通 Codex 或 Desktop 不会使用这份试用配置。', 'On demand · Codex CLI only. Use the command below in a terminal on the device with Codex installed to start a new session. Ordinary Codex and Desktop do not use this trial configuration.')
          : t('已沿用默认接入。重新启动此配置目录对应的 Codex CLI 或 Desktop，再开启新会话即可使用试用路由。', 'Your default connection is retained. Restart the Codex CLI or Desktop using this configuration directory, then start a new session to use the trial route.')}</p>
        <CodexLaunchInstructions access={access} language={language} primary />
        <CodexHistoryNotice language={language} mode={access.selected_mode} compact />
      </>}
  </section>;
}
