import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SkillResourceStatus } from './types';

type CliEntry = { state: 'missing' | 'valid' | 'broken' | 'conflict'; command_resolves_to_app_cli: boolean; path_contains_entry_directory: boolean };

/** Shares the existing Desktop CLI-entry preparation commands; no shell/profile writes. */
export function AgentSkillDetails({ resource, language, management = false }: {
  resource?: SkillResourceStatus; language: 'zh' | 'en'; management?: boolean;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [cli, setCli] = useState<CliEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    let current = true;
    if (management) void invoke<CliEntry>('cli_entry_status').then(value => { if (current) setCli(value); }).catch(() => { if (current) setError(true); });
    return () => { current = false; };
  }, [management]);
  const read = async () => {
    if (busy) return;
    setBusy(true); setError(false);
    try { setCli(await invoke<CliEntry>('cli_entry_status')); }
    catch { setError(true); }
    finally { setBusy(false); }
  };
  const prepare = async () => {
    if (busy) return;
    setBusy(true); setError(false);
    try { setCli(await invoke<CliEntry>('cli_entry_install')); }
    catch { setError(true); }
    finally { setBusy(false); }
  };
  return <div className="field-help" data-agent-skill-details>
    {resource && <>
      <p className="agent-resource-path">{text('Skill 目录：', 'Skill directory: ')}{resource.directory}</p>
      {resource.file_ownership === 'borrowed_identical' && <p>{text('使用已有的相同 Skill；恢复配置后会保留外部文件。', 'Uses an existing identical Skill; restoring settings retains the external file.')}</p>}
      {resource.shared_context_count > 1 && <p>{text('此 Skill 由多个配置共享，恢复只释放当前配置的引用。', 'Several configurations share this Skill. Restore releases only the current reference.')}</p>}
      {resource.native_available === null && <p>{text('暂时无法读取此 Skill 文件。请检查目录访问情况后重新扫描。', 'This Skill file cannot currently be read. Check directory access and scan again.')}</p>}
      {resource.native_available && <p>{text('Skill 文件可用；当前会话是否已加载尚未确认，必要时重新加载 Skill 或开启新会话。', 'Skill files are available. Loading in the current session is unconfirmed; reload Skills or start a new session if needed.')}</p>}
    </>}
    {management && cli && cli.state !== 'valid' && <p>
      {cli.state === 'conflict' ? text('本机命令行入口存在其他内容，请到设置中处理。', 'Another command occupies the local CLI entry. Resolve it in Settings.') : text('尚未准备本机命令行入口。', 'The local CLI entry is not prepared.')}
      {cli.state !== 'conflict' && <button className="btn btn-quiet" type="button" disabled={busy} onClick={() => void prepare()}>{busy ? text('准备中…', 'Preparing…') : text('准备命令行入口', 'Prepare CLI entry')}</button>}
    </p>}
    {management && cli?.state === 'valid' && !cli.command_resolves_to_app_cli && <p>{text('命令行入口已准备；Agent 的运行环境仍需能通过 PATH 找到该入口。', 'The CLI entry is prepared; the Agent environment must still resolve it on PATH.')}</p>}
    {error && <p role="status">{text('暂时无法确认命令行入口，可在设置中检查或重试。', 'The CLI entry could not be confirmed. Check it in Settings or retry.')}<button className="btn btn-quiet" type="button" disabled={busy} onClick={() => void read()}>{text('重试', 'Retry')}</button></p>}
  </div>;
}
