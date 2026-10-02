import React, { useState } from 'react';

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

export function CodexAccessPanel({ access, mode, language, disabled, onMode, onRetry }: {
  access: CodexAccess; mode: 'profile' | 'root'; language: string; disabled: boolean;
  onMode: (mode: 'profile' | 'root') => void; onRetry: () => Promise<void>;
}) {
  const [shell, setShell] = useState('bash/zsh');
  const [copyState, setCopyState] = useState('');
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const command = access.commands[shell];
  return <section className="fact-section" data-codex-access-mode={mode}>
    <h3>{text('Codex 接入方式', 'Codex connection mode')}</h3>
    <label className="field-label">{text('接入目标', 'Connection target')}
      <select aria-label={text('Codex 接入方式', 'Codex connection mode')} value={mode} disabled={disabled || access.slot_occupied} onChange={e => onMode(e.target.value as 'profile' | 'root')}>
        <option value="profile">{text('独立 CLI profile（推荐）', 'Separate CLI profile (recommended)')}</option>
        <option value="root">{text('默认配置接管（高级）', 'Take over default configuration (advanced)')}</option>
      </select>
    </label>
    <p>{mode === 'profile'
      ? text('通过指定 profile 使用 HiRoute。普通 Codex 和 Desktop 默认入口继续使用原来的 root 配置；不支持指定此 profile 启动 Desktop。', 'Use HiRoute through the selected CLI profile. Ordinary Codex and Desktop entry points keep their root configuration. Launching Desktop with this profile is not supported.')
      : text('此模式修改默认 provider，影响使用同一 CODEX_HOME 的普通 Codex CLI 和 Desktop。', 'This mode changes the default provider for ordinary Codex CLI and Desktop using the same CODEX_HOME.')}</p>
    <p>{text('各入口不保证显示相同历史，也不提供跨 provider 历史恢复。', 'History may differ between entry points; cross-provider history restoration is not provided.')}</p>
    <dl><div><dt>CODEX_HOME</dt><dd><code>{access.codex_home}</code></dd></div>
      {access.slot_occupied && <div><dt>{text('目标文件', 'Target file')}</dt><dd><code>{access.target_file}</code></dd></div>}
    </dl>
    {access.slot_occupied && <p>{text('切换方式：先撤销当前模型接入，文件恢复完成后再选择另一模式。', 'To switch modes, revoke this model connection and finish file restoration, then select the other mode.')}</p>}
    {mode === 'profile' && access.slot_occupied && !access.pending_operation && <div>
      <label className="field-label">Shell <select value={shell} onChange={e => { setShell(e.target.value); setCopyState(''); }}>{Object.keys(access.commands).map(name => <option key={name}>{name}</option>)}</select></label>
      <pre><code>{command}</code></pre>
      <button className="btn" disabled={!command} onClick={async () => {
        try { await navigator.clipboard.writeText(command); setCopyState(text('已复制', 'Copied')); }
        catch { setCopyState(text('复制失败，请手动选择命令复制', 'Copy failed; select and copy the command manually')); }
      }}>{text('复制启动命令', 'Copy launch command')}</button><span role="status">{copyState}</span>
    </div>}
    {access.pending_operation && <div className="callout warn" role="alert">
      <p>{access.access_revoked
        ? text('访问权限已撤销，配置文件尚未清理。', 'Access has been revoked; configuration cleanup is still pending.')
        : text('当前操作尚未完成，不能切换接入模式。', 'The current operation is incomplete; mode switching is blocked.')}</p>
      <p><code>{access.target_file}</code></p>
      {access.conflict_fields.length > 0 && <ul>{access.conflict_fields.map(field => <li key={field}><code>{field}</code></li>)}</ul>}
      <p>{access.access_revoked
        ? text('请修复所列冲突，或自行清理 HiRoute 受管字段后重新检查。无关设置和注释可以保留。清理完成前，新的配置保存会暂停。', 'Resolve the listed conflicts, or remove the HiRoute-managed fields and recheck. Unrelated settings and comments can remain. New configuration saves wait until cleanup finishes.')
        : text('请先保存自己的编辑，恢复操作开始后发生的文件改动，再重新检查。完成前新的配置保存会暂停。HiRoute 不会强制覆盖文件。', 'Save your edits, undo file changes made since this operation started, then recheck. New configuration saves wait until it finishes. HiRoute will not force an overwrite.')}</p>
      <button className="btn" disabled={disabled} onClick={() => void onRetry()}>{text('重新检查并继续原操作', 'Recheck and resume operation')}</button>
    </div>}
  </section>;
}
