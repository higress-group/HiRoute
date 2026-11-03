import { hostCapabilities } from '../../host/index.ts';
import {
  canSave,
  fixedFailureText,
  settingsReadErrorText,
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
            <button className="btn" type="button" onClick={() => void view.refresh()}>{text('重试', 'Retry')}</button>
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
              {hostCapabilities().directory_reveal === 'path' ? text('日志保存在服务所在设备。显示路径后，在该设备打开 daemon/ 目录查看 .jsonl 日志。日志不含对话正文或凭据。', 'Logs are stored on the service device. Show the path and open daemon/ there to view .jsonl files. Logs contain no conversation content or credentials.') : text(
                '打开本机日志目录：desktop/ 为客户端日志，daemon/ 为服务日志。向社区求助时可附上其中的 .jsonl 文件。日志不含对话正文或凭据。',
                'Open the local log directory: desktop/ holds client logs and daemon/ holds service logs. Attach the .jsonl files when asking the community for help. Logs contain no conversation content or credentials.',
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
