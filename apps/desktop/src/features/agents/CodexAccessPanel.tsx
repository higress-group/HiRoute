import React, { useState } from 'react';
import { Disclosure, UiIcon, copyText } from '../../ui';
import { preferredCodexShell } from './codex-launch';

export type { CodexAccess } from './types';
import type { CodexAccess } from './types';

export function CodexHistoryNotice({ language, mode }: { language: 'zh' | 'en'; mode: 'profile' | 'root' }) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const root = mode === 'root';
  return <div className={`callout${root ? ' warn' : ''}`} role="note" data-codex-history-guidance data-codex-history-mode={mode}>
    <UiIcon name={root ? 'warning' : 'info'} /><div>
      <strong>{root
        ? text('切换后如何找回原有会话', 'Finding earlier sessions after switching')
        : text('原有会话仍可从原入口继续', 'Continue earlier sessions from the original entry')}</strong>
      <p>{root
        ? text('设为默认后，Codex 按模型服务商筛选 /resume。原有会话可能不再显示，即使选择全部目录；会话文件仍然保留。', 'After using HiRoute by default, Codex filters /resume by model provider. Earlier sessions may disappear, even with all directories selected; their files remain.')
        : text('按需使用只影响 HiRoute profile，普通 Codex 的配置和会话历史不变。HiRoute profile 内的 /resume 可能不显示原服务商的会话。', 'On-demand access only affects the HiRoute profile. Ordinary Codex configuration and session history stay unchanged. Inside the HiRoute profile, /resume may hide sessions from the original provider.')}</p>
      <p>{root
        ? text('找回步骤：在 HiRoute 的“模型路由”点击“停用”，等待原配置恢复完成，再使用原 CODEX_HOME 和原 profile 运行 ', 'To recover: turn off Model routing in HiRoute and wait for configuration restoration to finish. Then use your original CODEX_HOME and profile to run ')
        : text('无需停用 HiRoute。退出 HiRoute profile，用原 CODEX_HOME 和原 profile 运行 ', 'No need to disable HiRoute. Exit the HiRoute profile and use your original CODEX_HOME and profile to run ')}<code>codex resume --all</code>{text('，选择原有会话。', ' and select the earlier session.')}</p>
      <p>{text('原入口使用原服务商，绕过 HiRoute，需要原账号或 API 凭据。', 'The original entry uses the original provider, bypasses HiRoute, and needs its original account or API credentials.')}</p>
    </div>
  </div>;
}

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
      <dl><div><dt>CODEX_HOME</dt><dd><code className="configuration-path">{access.codex_home}</code></dd></div>
        {access.slot_occupied && <div><dt>{text('目标文件', 'Target file')}</dt><dd><code className="configuration-path">{access.target_file}</code></dd></div>}
      </dl>
      <p className="field-help">{text('使用原有会话时需选择相同的 CODEX_HOME 和原 profile。独立 profile 不支持启动 Desktop。', 'Use the same CODEX_HOME and original profile for earlier sessions. The separate profile cannot launch Desktop.')}</p>
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
    {mode === 'profile' && access.slot_occupied && !access.pending_operation && !access.access_revoked && <div className="agent-launch-row">
      <div><strong>{text('终端启动', 'Start in a terminal')}</strong><p>{text('复制命令，开启使用 HiRoute 的 CLI 会话。', 'Copy the command to start a CLI session using HiRoute.')}</p></div>
      <button className="btn" disabled={!command} onClick={async () => {
        try { await copyText(command); setCopyState(text('已复制', 'Copied')); }
        catch { setCopyState(text('复制失败，请手动选择命令复制', 'Copy failed; select and copy the command manually')); }
      }}>{copyState || text('复制启动命令', 'Copy launch command')}</button>
    </div>}
    {mode === 'root' && !access.access_revoked && <p className="field-help" data-codex-history-summary>{text('当前为默认接入。原有会话可能被服务商筛选隐藏，可停用模型路由恢复原配置。', 'This is the default connection. Provider filtering may hide earlier sessions; disable model routing to restore the original settings.')}</p>}
    <Disclosure className="agent-launch-help" label={mode === 'profile' ? text('启动命令与会话说明', 'Launch command and sessions') : text('原有会话与配置说明', 'Earlier sessions and settings')} language={language}>
      {mode === 'profile' && <><label className="field agent-launch-shell"><span className="field-label">Shell</span><select className="select" value={shell} onChange={e => { setShell(e.target.value); setCopyState(''); }}>{Object.keys(access.commands).map(name => <option key={name}>{name}</option>)}</select></label><pre className="agent-launch-command"><code>{command}</code></pre></>}
      <CodexHistoryNotice language={language} mode={mode} />
    </Disclosure>
    {!access.pending_operation && !access.access_revoked && access.conflict_fields.length > 0 && <div className="callout warn" role="alert" data-codex-restore-conflict>
      <UiIcon name="warning" /><div>
      <strong>{text('配置文件存在冲突', 'Configuration file conflict')}</strong>
      <p>{text('文件冲突解决前无法完成停用，原连接仍有效。HiRoute 会保留你的修改；请处理以下字段后重新点击“停用”。其他配置仍可修改。', 'Disable cannot complete until the conflict is resolved; the original connection remains active. HiRoute preserves your edits. Resolve these fields, then click Disable again. Other settings remain editable.')}</p>
      <p><code className="configuration-path">{access.target_file}</code></p>
      <ul>{access.conflict_fields.map(field => <li key={field}><code>{field}</code></li>)}</ul>
      </div>
    </div>}
    {access.pending_operation && <div className="callout warn" role="alert">
      <UiIcon name="warning" /><div>
      <p>{access.access_revoked
        ? text('已停用，配置文件待清理。旧令牌已失效。', 'Disabled; configuration cleanup is pending. The old token is invalid.')
        : text('当前操作尚未完成，不能切换接入模式。', 'The current operation is incomplete; mode switching is blocked.')}</p>
      <p><code className="configuration-path">{access.target_file}</code></p>
      {access.conflict_fields.length > 0 && <ul>{access.conflict_fields.map(field => <li key={field}><code>{field}</code></li>)}</ul>}
      <p>{access.access_revoked
        ? text('请修复所列冲突，或自行清理 HiRoute 受管字段后重新检查。无关设置和注释可以保留。清理完成前，新的配置保存会暂停。', 'Resolve the listed conflicts, or remove the HiRoute-managed fields and recheck. Unrelated settings and comments can remain. New configuration saves wait until cleanup finishes.')
        : text('请先保存自己的编辑，恢复操作开始后发生的文件改动，再重新检查。完成前新的配置保存会暂停。HiRoute 不会强制覆盖文件。', 'Save your edits, undo file changes made since this operation started, then recheck. New configuration saves wait until it finishes. HiRoute will not force an overwrite.')}</p>
      <button className="btn" disabled={retryDisabled} onClick={() => void onRetry()}>{text('重新检查并继续原操作', 'Recheck and resume operation')}</button>
      </div>
    </div>}
  </section>;
}
