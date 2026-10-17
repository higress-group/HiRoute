import { hostCapabilities } from '../../host/index.ts';
import {
  canSave,
  debugEnabled,
  fixedFailureText,
  settingsReadErrorText,
  toggleDebug,
} from './diagnostics-state';
import { useDiagnostics } from './use-diagnostics';
import { DIAGNOSTIC_LEVELS, type DiagnosticLevel } from './types';

type DiagnosticsSettingsProps = {
  active: boolean;
  language: 'zh' | 'en';
};

const LEVEL_LABELS: Record<DiagnosticLevel, { zh: string; en: string }> = {
  error: { zh: '仅错误', en: 'Errors only' },
  warn: { zh: '警告', en: 'Warnings' },
  info: { zh: '信息（默认）', en: 'Info (default)' },
  debug: { zh: '调试', en: 'Debug' },
};

/**
 * One self-contained section: it reads and writes the native diagnostics state and never
 * depends on a business snapshot, so it keeps working while the local service is starting
 * or failed. Every result is announced through a live region with fixed localized text.
 */
export function DiagnosticsSettings(props: DiagnosticsSettingsProps) {
  const text = (zh: string, en: string) => (props.language === 'zh' ? zh : en);
  const view = useDiagnostics(props.active, props.language);
  const status = view.status;
  const readError = settingsReadErrorText(status, props.language);

  return (
    <section className="settings-group" aria-labelledby="diagnostics-heading">
      <h2 id="diagnostics-heading">{text('诊断日志', 'Diagnostic logs')}</h2>
      <div className="settings-list">
        <div className="settings-row">
          <div className="settings-copy">
            <strong>{text('日志等级', 'Log level')}</strong>
            <span>
              {text(
                '仅保存在本机；提高等级不会记录对话内容、凭据或请求正文。',
                'Stored locally only; a higher level still never records conversation content, credentials or request bodies.',
              )}
            </span>
          </div>
          <div className="diagnostics-controls">
            <label className="diagnostics-toggle">
              <input
                type="checkbox"
                checked={debugEnabled(view.draft)}
                aria-label={text('开启调试日志', 'Enable debug logs')}
                onChange={event => view.setDraft(toggleDebug(view.draft, event.target.checked))}
              />
              <span>{text('开启调试日志', 'Enable debug logs')}</span>
            </label>
            <label className="diagnostics-select">
              <span>{text('等级', 'Level')}</span>
              <select
                value={view.draft}
                disabled={view.saving}
                aria-label={text('日志等级', 'Log level')}
                onChange={event => view.setDraft(event.target.value as DiagnosticLevel)}
              >
                {DIAGNOSTIC_LEVELS.slice()
                  .reverse()
                  .map(level => (
                    <option key={level} value={level}>
                      {LEVEL_LABELS[level][props.language]}
                    </option>
                  ))}
              </select>
            </label>
            <button
              className="btn"
              type="button"
              disabled={!canSave(status, view.draft) || view.saving}
              onClick={() => void view.save()}
            >
              {view.saving
                ? text('保存中…', 'Saving…')
                : text('保存等级', 'Save level')}
            </button>
          </div>
        </div>
        {readError && (
          <p className="diagnostics-note error" role="alert">
            {readError}
          </p>
        )}
        {view.saveError && (
          <p className="diagnostics-note error" role="alert">
            {text('保存失败：', 'Save failed: ')}
            {fixedFailureText(view.saveError, props.language)}
            <button className="btn" type="button" onClick={() => void view.refresh()}>
              {text('刷新', 'Refresh')}
            </button>
          </p>
        )}
        <div className="settings-row">
          <div className="settings-copy">
            <strong>{text('日志目录', 'Log directory')}</strong>
            <span>
              {hostCapabilities().directory_reveal === 'path' ? text('日志保存在运行 HiRoute 服务的设备上。可显示目录路径，在该设备上打开目录并查看 daemon/ 下的日志文件；日志不包含对话内容或凭据。', 'Logs are stored on the device running HiRoute. Show the directory path and open it on that device to read the daemon/ logs. Logs contain no conversation content or credentials.') : text(
                '日志保存在本机应用数据目录；点击会在文件管理器中打开该目录。目录里的 .jsonl 文件就是日志：desktop/ 下是客户端日志，daemon/ 下是引擎（本机服务）日志；向社区求助时附上这些文件即可，其余文件不需要发送。日志不含对话内容或凭据。',
                'The logs live in this app\'s local data directory. The button opens that folder in your file manager. The .jsonl files are the logs: desktop/ holds the client logs and daemon/ the engine (local service) logs; attach these when asking the community for help — the other files are not needed. The logs never contain conversation content or credentials.',
              )}
            </span>
          </div>
          <div className="diagnostics-controls">
            <button
              className="btn"
              type="button"
              disabled={!status?.logs_directory_available || view.logsState.phase === 'busy'}
              onClick={() => void view.openLogs()}
            >
              {view.logsState.phase === 'busy'
                ? text('打开中…', 'Opening…')
                : hostCapabilities().directory_reveal === 'path' ? text('显示目录路径', 'Show directory path') : text('打开日志目录', 'Open log directory')}
            </button>
          </div>
        </div>
        <p className="diagnostics-note" aria-live="polite">
          {view.logsState.code ?? ''}
        </p>
      </div>
    </section>
  );
}
