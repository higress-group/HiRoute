import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { SubscriptionProxySettings } from '../features/SubscriptionProxySettings';
import { DiagnosticsSettings } from '../features/diagnostics/DiagnosticsSettings';
import { UpdatesSettings } from '../features/UpdatesSettings';
import { safeDiagnosticCode } from '../error-code';
import { ProductPage } from '../ui';
import type { Language, LanguagePreference, TextScale, ThemePreference } from '../ui/preferences';
import { listenerApplyRequest, listenerScopeFromAddress, type GatewayListenerScope } from './gateway-listener-scope';

type SettingsPageProps = {
  active: boolean; language: Language; languagePreference: LanguagePreference; theme: ThemePreference; textScale: TextScale;
  serviceLabel: string; serviceReady: boolean;
  onLanguageChange: (language: LanguagePreference) => void;
  onThemeChange: (theme: ThemePreference) => void;
  onTextScaleChange: (scale: TextScale) => void;
  onOpenSessions: (scope?: 'content_only' | 'facts_and_content') => void;
};

type WorkerSettings = {
  schema: 'hiroute.worker-settings/v1';
  max_concurrent: number;
};

type CliEntryView = {
  state: 'missing' | 'valid' | 'broken' | 'conflict';
  entry_path: string;
  app_cli_path?: string;
  path_contains_entry_directory: boolean;
  resolved_command?: string;
  command_resolves_to_app_cli: boolean;
  daemon_available: boolean;
};

type GatewayListenerView = {
  config: {
    schema_version: 'hiroute.gateway-listener/v1';
    desired: { address: string; port_mode: 'automatic' | 'fixed'; port?: number };
    applied?: { address: string; port: number; applied_at_unix: number };
    operation?: { state: 'submitted' | 'restarting' | 'succeeded' | 'failed'; error_code?: string };
  };
  connect_address?: string;
  suggested_agent_base_url?: string;
  existing_agent_count?: number;
  ready: boolean;
};

function validWorkerSettings(value: WorkerSettings): boolean {
  return value.schema === 'hiroute.worker-settings/v1'
    && Number.isInteger(value.max_concurrent)
    && value.max_concurrent >= 1
    && value.max_concurrent <= 1000;
}

/** occami-settings.js DOM, backed by real persisted preferences and service state. */
export function SettingsPage(props: SettingsPageProps) {
  const text = (zh: string, en: string) => props.language === 'zh' ? zh : en;
  const [workerLimit, setWorkerLimit] = useState<number | null>(null);
  const [workerDraft, setWorkerDraft] = useState('');
  const [workerBusy, setWorkerBusy] = useState(false);
  const [workerError, setWorkerError] = useState('');
  const [cliEntry, setCliEntry] = useState<CliEntryView | null>(null);
  const [cliBusy, setCliBusy] = useState(false);
  const [cliError, setCliError] = useState('');
  const [listener, setListener] = useState<GatewayListenerView | null>(null);
  const [listenerScope, setListenerScope] = useState<GatewayListenerScope>('local');
  const [listenerBusy, setListenerBusy] = useState(false);
  const [listenerError, setListenerError] = useState('');

  useEffect(() => {
    if (!props.active) return;
    let current = true;
    setWorkerBusy(true);
    setWorkerError('');
    void invoke<WorkerSettings>('worker_settings_get').then(value => {
      if (!current) return;
      if (!validWorkerSettings(value)) throw new Error('RESPONSE_DATA_INVALID');
      setWorkerLimit(value.max_concurrent);
      setWorkerDraft(String(value.max_concurrent));
    }).catch(error => {
      if (current) setWorkerError(safeDiagnosticCode(error, 'WORKER_SETTINGS_UNAVAILABLE'));
    }).finally(() => {
      if (current) setWorkerBusy(false);
    });
    return () => { current = false; };
  }, [props.active]);

  useEffect(() => {
    if (!props.active) return;
    let current = true;
    setCliError('');
    setListenerError('');
    void invoke<CliEntryView>('cli_entry_status').then(value => {
      if (current) setCliEntry(value);
    }).catch(error => {
      if (current) setCliError(safeDiagnosticCode(error, 'CLI_ENTRY_UNAVAILABLE'));
    });
    void invoke<GatewayListenerView>('gateway_listener_status').then(value => {
      if (!current) return;
      setListener(value);
      setListenerScope(listenerScopeFromAddress(value.config.desired.address));
    }).catch(error => {
      if (current) setListenerError(safeDiagnosticCode(error, 'GATEWAY_LISTENER_CONFIG_UNAVAILABLE'));
    });
    return () => { current = false; };
  }, [props.active]);

  const updateCliEntry = async (command: 'cli_entry_install' | 'cli_entry_remove') => {
    setCliBusy(true);
    setCliError('');
    try {
      setCliEntry(await invoke<CliEntryView>(command));
    } catch (error) {
      setCliError(safeDiagnosticCode(error, 'CLI_ENTRY_UNAVAILABLE'));
    } finally {
      setCliBusy(false);
    }
  };

  const saveListener = async () => {
    if (!listener) return;
    setListenerBusy(true);
    setListenerError('');
    try {
      await invoke('gateway_listener_apply', { input: listenerApplyRequest(listenerScope) });
    } catch (error) {
      setListenerError(safeDiagnosticCode(error, 'GATEWAY_LISTENER_CONFIG_UNAVAILABLE'));
      setListenerBusy(false);
    }
  };

  const recoverListener = async () => {
    setListenerBusy(true);
    setListenerError('');
    try {
      await invoke('gateway_listener_recover');
    } catch (error) {
      setListenerError(safeDiagnosticCode(error, 'GATEWAY_LISTENER_CONFIG_UNAVAILABLE'));
      setListenerBusy(false);
    }
  };

  const saveWorkerLimit = async () => {
    const maxConcurrent = Number(workerDraft);
    if (!Number.isInteger(maxConcurrent) || maxConcurrent < 1 || maxConcurrent > 1000) {
      setWorkerError('INVALID_WORKER_CONCURRENCY');
      return;
    }
    setWorkerBusy(true);
    setWorkerError('');
    try {
      const value = await invoke<WorkerSettings>('worker_settings_set', {
        input: { schema: 'hiroute.worker-settings/v1', max_concurrent: maxConcurrent },
      });
      if (!validWorkerSettings(value)) throw new Error('RESPONSE_DATA_INVALID');
      setWorkerLimit(value.max_concurrent);
      setWorkerDraft(String(value.max_concurrent));
    } catch (error) {
      setWorkerError(safeDiagnosticCode(error, 'WORKER_SETTINGS_UNAVAILABLE'));
    } finally {
      setWorkerBusy(false);
    }
  };

  const listenerErrorMessage = (code: string) => {
    if (code === 'REMOTE_LISTENER_CONFIRMATION_REQUIRED') return text('对外开放未获确认，设置没有改变。请重试；若仍出现，请反馈此错误。', 'Network access was not confirmed, so nothing changed. Retry and report this error if it persists.');
    if (code === 'GATEWAY_LISTENER_CONFIG_INVALID') return text('监听器配置无效。可恢复上次已应用的设置后重试。', 'The listener configuration is invalid. Recover the last applied setting and retry.');
    return text(`监听器设置不可用（${code}）`, `Listener settings unavailable (${code})`);
  };

  return <ProductPage title={text('设置', 'Settings')} subtitle={text('偏好与本机服务', 'Preferences and local service')}>
    <div className="settings-wrap">
      <section className="settings-group"><h2>{text('偏好', 'Preferences')}</h2><div className="settings-list">
        <div className="settings-row"><div className="settings-copy"><strong>{text('语言', 'Language')}</strong><span>{text('界面与状态文案', 'Interface and status text')}</span></div>
          <div className="segmented">{(['system', 'zh', 'en'] as const).map(language => <button type="button" key={language} className={`segment ${props.languagePreference === language ? 'active' : ''}`} aria-pressed={props.languagePreference === language} onClick={() => props.onLanguageChange(language)}>{language === 'system' ? text('跟随系统', 'System') : language === 'zh' ? '中文' : 'English'}</button>)}</div>
        </div>
        <div className="settings-row"><div className="settings-copy"><strong>{text('外观', 'Appearance')}</strong><span>{text('默认跟随系统，可随时切换', 'Follows the system by default')}</span></div>
          <div className="segmented">{(['system', 'light', 'dark'] as const).map(theme => <button type="button" key={theme} className={`segment ${props.theme === theme ? 'active' : ''}`} aria-pressed={props.theme === theme} onClick={() => props.onThemeChange(theme)}>{theme === 'system' ? text('跟随系统', 'System') : theme === 'light' ? text('浅色', 'Light') : text('深色', 'Dark')}</button>)}</div>
        </div>
        <div className="settings-row"><div className="settings-copy"><strong>{text('文字大小', 'Text size')}</strong><span>{text('标题、正文、提示和控件一起缩放', 'Scales headings, body text, messages, and controls together')}</span></div>
          <div className="segmented">{([1, 1.5, 2] as const).map(scale => <button type="button" key={scale} className={`segment ${props.textScale === scale ? 'active' : ''}`} aria-pressed={props.textScale === scale} onClick={() => props.onTextScaleChange(scale)}>{Math.round(scale * 100)}%</button>)}</div>
        </div>
      </div></section>
      <section className="settings-group"><h2>Worker</h2><div className="settings-list"><div className="settings-row">
        <div className="settings-copy"><strong>{text('并发任务上限', 'Concurrent task limit')}</strong><span>{workerLimit === null ? text('允许 1–1000 个任务', 'Allows 1–1000 tasks') : text(`当前生效：${workerLimit}`, `Effective now: ${workerLimit}`)}</span></div>
        <div className="worker-settings-control">
          <input className="input worker-settings-input" type="number" min={1} max={1000} step={1} inputMode="numeric" aria-label={text('并发任务上限', 'Concurrent task limit')} value={workerDraft} disabled={workerBusy} onChange={event => setWorkerDraft(event.target.value)} />
          <button className="btn" type="button" disabled={workerBusy || workerDraft === '' || workerDraft === String(workerLimit)} onClick={() => void saveWorkerLimit()}>{workerBusy ? text('处理中…', 'Working…') : text('保存', 'Save')}</button>
        </div>
      </div>{workerError && <div className="settings-inline-error" role="status">{workerError === 'INVALID_WORKER_CONCURRENCY' ? text('请输入 1 到 1000 之间的整数。', 'Enter an integer from 1 to 1000.') : text(`设置暂时不可用（${workerError}）`, `Settings unavailable (${workerError})`)}</div>}</div></section>
      <section className="settings-group"><h2>{text('本地记录', 'Local records')}</h2><div className="settings-list"><div className="settings-row">
        <div className="settings-copy"><strong>{text('会话内容', 'Session content')}</strong><span>{text('在会话详情中，可以清理该会话的正文或记录。', 'Clear content or records from each session’s detail view.')}</span></div>
        <button className="btn" type="button" onClick={() => props.onOpenSessions()}>{text('查看会话', 'View sessions')}</button>
      </div></div></section>
      <SubscriptionProxySettings active={props.active} language={props.language} />
      <DiagnosticsSettings active={props.active} language={props.language} />
      <UpdatesSettings active={props.active} language={props.language} />
      <section className="settings-group"><h2>CLI</h2><div className="settings-list"><div className="settings-row">
        <div className="settings-copy"><strong>{text('终端入口', 'Terminal entry')}</strong><span>{cliEntry === null
          ? text('正在检查 ~/.local/bin/hiroute', 'Checking ~/.local/bin/hiroute')
          : text(`状态：${cliEntry.state} · ${cliEntry.entry_path}`, `State: ${cliEntry.state} · ${cliEntry.entry_path}`)}</span>
          {cliEntry?.state === 'valid' && !cliEntry.path_contains_entry_directory && <span>{text('zsh：将 export PATH="$HOME/.local/bin:$PATH" 加入 ~/.zprofile 后重新打开终端；HiRoute 不会代写。', 'zsh: add export PATH="$HOME/.local/bin:$PATH" to ~/.zprofile, then reopen Terminal. HiRoute does not edit it.')}</span>}
          {cliEntry?.path_contains_entry_directory && !cliEntry.command_resolves_to_app_cli && <span>{text(`当前被更早的命令遮蔽：${cliEntry.resolved_command ?? 'unknown'}`, `An earlier command shadows this entry: ${cliEntry.resolved_command ?? 'unknown'}`)}</span>}
          {cliEntry && !cliEntry.daemon_available && <span>{text('CLI 入口与服务可用性相互独立；当前本机服务尚未就绪。', 'The CLI entry and daemon availability are independent; the local service is not ready.')}</span>}
        </div>
        <div className="worker-settings-control">
          {cliEntry?.state !== 'valid' && <button className="btn" type="button" disabled={cliBusy || cliEntry?.state === 'conflict'} onClick={() => void updateCliEntry('cli_entry_install')}>{cliEntry?.state === 'broken' ? text('修复', 'Repair') : text('安装', 'Install')}</button>}
          {cliEntry?.state === 'valid' && <button className="btn" type="button" disabled={cliBusy} onClick={() => void updateCliEntry('cli_entry_remove')}>{text('移除', 'Remove')}</button>}
        </div>
      </div>{cliError && <div className="settings-inline-error" role="status">{text(`CLI 入口不可用（${cliError}）`, `CLI entry unavailable (${cliError})`)}</div>}</div></section>
      <section className="settings-group"><h2>Gateway</h2><div className="settings-list"><div className="settings-row">
        <div className="settings-copy"><strong>{text('监听范围', 'Listener access')}</strong><span>{listener?.config.applied
          ? text(`已应用 ${listener.config.applied.address}:${listener.config.applied.port} · ${listener.ready ? 'ready' : 'not ready'}`, `Applied ${listener.config.applied.address}:${listener.config.applied.port} · ${listener.ready ? 'ready' : 'not ready'}`)
          : text('尚无已应用地址', 'No applied address yet')}</span>{listener?.connect_address && <span>{text(`本机连接：${listener.connect_address}`, `Local connect address: ${listener.connect_address}`)}</span>}</div>
        <div className="gateway-listener-control">
          <div className="segmented" role="group" aria-label={text('监听范围', 'Listener access')}>
            {(['local', 'external'] as const).map(scope => <button type="button" key={scope} className={`segment ${listenerScope === scope ? 'active' : ''}`} aria-pressed={listenerScope === scope} disabled={!listener || listenerBusy} onClick={() => { setListenerScope(scope); setListenerError(''); }}>{scope === 'local' ? text('仅本机', 'This device') : text('对外开放', 'Network access')}</button>)}
          </div>
          <button className="btn" type="button" disabled={!listener || listenerBusy} onClick={() => void saveListener()}>{listenerBusy ? text('正在重启…', 'Restarting…') : text('保存并重启', 'Save and restart')}</button>
          {(listener?.config.operation?.state === 'failed' || listenerError) && <button className="btn" type="button" disabled={listenerBusy} onClick={() => void recoverListener()}>{text('恢复上次设置', 'Recover previous setting')}</button>}
        </div>
      </div>{listenerScope === 'external' && listener && <div className="gateway-risk">{text('对外开放会监听所有 IPv4 网卡；点击“保存并重启”即确认扩大网络访问范围。仍需自行配置防火墙、TLS 和 Agent，现有请求鉴权保持启用。', 'Network access listens on all IPv4 interfaces. Save and restart confirms the wider exposure. Firewall, TLS, and Agent setup remain your responsibility; request authorization stays enabled.')}</div>}{listener?.suggested_agent_base_url && (listener.existing_agent_count ?? 0) > 0 && <div className="gateway-risk">{text(`发现 ${listener.existing_agent_count} 个 Agent。监听范围改变后，如需从其他设备使用，请在 Agent 设置中检查连接地址 ${listener.suggested_agent_base_url}；现有配置不会自动覆盖。`, `${listener.existing_agent_count} Agent(s) detected. To use them from another device after changing listener access, check their connection address ${listener.suggested_agent_base_url} in Agent settings. Existing settings are not overwritten.`)}</div>}{listener?.config.operation?.state === 'failed' && <div className="settings-inline-error" role="status">{text(`上次应用失败（${listener.config.operation.error_code ?? 'UNKNOWN'}），已应用地址保持不变。`, `The last apply failed (${listener.config.operation.error_code ?? 'UNKNOWN'}); the applied address is unchanged.`)}</div>}{listenerError && <div className="settings-inline-error" role="status">{listenerErrorMessage(listenerError)}</div>}</div></section>
      <section className="settings-group"><h2>{text('服务', 'Service')}</h2><div className="settings-list"><div className="settings-row">
        <div className="settings-copy"><strong>{text('本机服务', 'Local service')}</strong><span>{text('模型与 Agent 路由使用本机服务', 'Model and agent routing use the local service')}</span></div>
        <span className={`badge ${props.serviceReady ? 'good' : 'warn'}`}>{props.serviceLabel}</span>
      </div></div></section>
    </div>
  </ProductPage>;
}
