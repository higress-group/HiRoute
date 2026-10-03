import React, { useState } from 'react';
import { Disclosure, copyText } from '../../ui';
import { preferredCodexShell } from './codex-launch';

export type CodexAccess = {
  codex_home: string;
  slot_id: string;
  profile_context_id: string;
  root_context_id: string;
  selected_mode: 'profile' | 'root';
  slot_occupied: boolean;
  target_file: string;
  profile_name: string;
  commands: Record<string, string>;
  pending_operation: string | null;
  access_revoked: boolean;
  conflict_fields: string[];
};

export function CodexAccessSettings({ access, mode, language, disabled, onMode }: {
  access: CodexAccess; mode: 'profile' | 'root'; language: 'zh' | 'en'; disabled: boolean;
  onMode: (mode: 'profile' | 'root') => void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  return <div data-codex-access-mode={mode}>
    <label className="field"><span className="field-label">{text('接入方式', 'Connection mode')}</span>
      <select className="select" aria-label={text('Codex 接入方式', 'Codex connection mode')} value={mode} disabled={disabled || access.slot_occupied} onChange={e => onMode(e.target.value as 'profile' | 'root')}>
        <option value="profile">{text('按需使用 · 仅 CLI（推荐）', 'On demand · CLI only (recommended)')}</option>
        <option value="root">{text('设为默认 · CLI 和 Desktop（高级）', 'Use by default · CLI and Desktop (advanced)')}</option>
      </select>
    </label>
    {access.slot_occupied && <p className="field-help">{text('切换接入方式前，请先停用并完成配置恢复。', 'Disable and finish restoring the configuration before switching modes.')}</p>}
    <Disclosure label={text('配置位置与会话历史', 'Configuration location and history')} language={language}>
      <dl><div><dt>CODEX_HOME</dt><dd><code>{access.codex_home}</code></dd></div>
        {access.slot_occupied && <div><dt>{text('目标文件', 'Target file')}</dt><dd><code>{access.target_file}</code></dd></div>}
      </dl>
      <p className="field-help">{text('不同入口的会话历史可能不同；不提供跨 provider 历史恢复。独立 profile 不支持启动 Desktop。', 'History may differ between entry points; cross-provider recovery is not provided. The separate profile cannot launch Desktop.')}</p>
    </Disclosure>
  </div>;
}

export function CodexAccessPanel({ access, language, retryDisabled, onRetry }: {
  access: CodexAccess; language: 'zh' | 'en'; retryDisabled: boolean; onRetry: () => Promise<void>;
}) {
  const [shell, setShell] = useState(() => preferredCodexShell(typeof navigator === 'undefined' ? '' : navigator.platform, access.commands));
  const [copyState, setCopyState] = useState('');
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (!access.slot_occupied && !access.pending_operation) return null;
  const mode = access.selected_mode;
  const command = access.commands[shell];
  return <section className="fact-section" data-codex-access-mode={mode}>
    <p className="field-help">{mode === 'profile'
      ? text('按需使用 · Codex CLI：复制命令到终端启动。普通 Codex 与 Desktop 保持原配置。', 'On demand · Codex CLI: copy the command to start in a terminal. Ordinary Codex and Desktop keep their settings.')
      : text('默认接入 · Codex CLI 和 Desktop：共享此配置目录的入口使用 HiRoute。', 'Default connection · Codex CLI and Desktop: entry points sharing this directory use HiRoute.')}</p>
    {mode === 'profile' && access.slot_occupied && !access.pending_operation && !access.access_revoked && <div>
      <div className="detail-section-head"><strong>{text('启动 Codex', 'Start Codex')}</strong><label className="field-label agent-launch-shell">Shell <select className="select" value={shell} onChange={e => { setShell(e.target.value); setCopyState(''); }}>{Object.keys(access.commands).map(name => <option key={name}>{name}</option>)}</select></label></div>
      <pre className="agent-launch-command"><code>{command}</code></pre>
      <button className="btn" disabled={!command} onClick={async () => {
        try { await copyText(command); setCopyState(text('已复制', 'Copied')); }
        catch { setCopyState(text('复制失败，请手动选择命令复制', 'Copy failed; select and copy the command manually')); }
      }}>{text('复制启动命令', 'Copy launch command')}</button><span role="status">{copyState}</span>
    </div>}
    {!access.pending_operation && !access.access_revoked && access.conflict_fields.length > 0 && <div className="callout warn" role="alert" data-codex-restore-conflict>
      <strong>{text('配置文件存在冲突', 'Configuration file conflict')}</strong>
      <p>{text('文件冲突解决前无法完成停用，原连接仍有效。HiRoute 会保留你的修改；请处理以下字段后重新点击“停用”。其他配置仍可修改。', 'Disable cannot complete until the conflict is resolved; the original connection remains active. HiRoute preserves your edits. Resolve these fields, then click Disable again. Other settings remain editable.')}</p>
      <p><code>{access.target_file}</code></p>
      <ul>{access.conflict_fields.map(field => <li key={field}><code>{field}</code></li>)}</ul>
    </div>}
    {access.pending_operation && <div className="callout warn" role="alert">
      <p>{access.access_revoked
        ? text('已停用，配置文件待清理。旧令牌已失效。', 'Disabled; configuration cleanup is pending. The old token is invalid.')
        : text('当前操作尚未完成，不能切换接入模式。', 'The current operation is incomplete; mode switching is blocked.')}</p>
      <p><code>{access.target_file}</code></p>
      {access.conflict_fields.length > 0 && <ul>{access.conflict_fields.map(field => <li key={field}><code>{field}</code></li>)}</ul>}
      <p>{access.access_revoked
        ? text('请修复所列冲突，或自行清理 HiRoute 受管字段后重新检查。无关设置和注释可以保留。清理完成前，新的配置保存会暂停。', 'Resolve the listed conflicts, or remove the HiRoute-managed fields and recheck. Unrelated settings and comments can remain. New configuration saves wait until cleanup finishes.')
        : text('请先保存自己的编辑，恢复操作开始后发生的文件改动，再重新检查。完成前新的配置保存会暂停。HiRoute 不会强制覆盖文件。', 'Save your edits, undo file changes made since this operation started, then recheck. New configuration saves wait until it finishes. HiRoute will not force an overwrite.')}</p>
      <button className="btn" disabled={retryDisabled} onClick={() => void onRetry()}>{text('重新检查并继续原操作', 'Recheck and resume operation')}</button>
    </div>}
  </section>;
}
