import type {
  DiagnosticLevel,
  DiagnosticsStatus,
} from './types';

/**
 * A newer or equal status is applied; an older revision is dropped so a late response can
 * never roll the accepted revision, level or save error back.
 */
export function acceptStatus(
  previous: DiagnosticsStatus | null,
  next: DiagnosticsStatus,
): DiagnosticsStatus {
  if (previous === null) return next;
  if (next.settings.revision < previous.settings.revision) return previous;
  return next;
}

/**
 * One response produces both the visible status and the draft, and both come from the accepted
 * snapshot: a rejected older response must not move the draft either. `keepDraft` carries an
 * unsaved user choice; `null` means the draft follows the accepted snapshot.
 */
export function acceptResponse(
  previous: DiagnosticsStatus | null,
  next: DiagnosticsStatus,
  keepDraft: DiagnosticLevel | null,
): { status: DiagnosticsStatus; draft: DiagnosticLevel } {
  const status = acceptStatus(previous, next);
  return { status, draft: keepDraft ?? status.settings.level };
}

/** The exact save body, or null when the draft already matches the saved revision. */
export function saveRequest(
  status: DiagnosticsStatus,
  draft: DiagnosticLevel,
): { expected_revision: number; level: DiagnosticLevel } | null {
  if (status.settings.level === draft) return null;
  return { expected_revision: status.settings.revision, level: draft };
}

export function canSave(status: DiagnosticsStatus | null, draft: DiagnosticLevel): boolean {
  if (!status) return false;
  return saveRequest(status, draft) !== null;
}

export type LogsState = { phase: 'idle' | 'busy' | 'opened' | 'failed'; code?: string };

/** The native failure codes this page can receive, in its own vocabulary. */
const FAILURE_CODES: Record<string, { zh: string; en: string }> = {
  override_active: {
    zh: '当前为临时覆盖等级，未保存。',
    en: 'A temporary override level is active; nothing was saved.',
  },
  settings_conflict: { zh: '已有更新的保存，请刷新后重试。', en: 'A newer save exists; refresh and try again.' },
  settings_busy: { zh: '另一处正在保存，请稍后重试。', en: 'Another save is in progress; try again.' },
  settings_invalid: { zh: '本机设置文件已损坏，未覆盖。', en: 'The local settings file is damaged; it was not overwritten.' },
  settings_unwritable: { zh: '无法写入本机设置。', en: 'The local settings cannot be written.' },
  path_unsafe: { zh: '本机日志位置不安全，已拒绝。', en: 'The local log location is not safe; the action was refused.' },
  unsupported_platform: { zh: '当前平台不支持该操作。', en: 'This action is unavailable on this platform.' },
  diagnostics_unavailable: {
    zh: '诊断日志当前不可用。',
    en: 'Diagnostic logs are currently unavailable.',
  },
  window_denied: { zh: '窗口校验失败。', en: 'The window check failed.' },
};

export function fixedFailureText(code: string, language: 'zh' | 'en'): string {
  const known = FAILURE_CODES[code];
  if (known) return known[language];
  return language === 'zh' ? '操作失败' : 'The action failed';
}

const NATIVE_CODE = /^[a-z][a-z0-9_]{2,60}$/u;

/**
 * The native failure code of a rejected command. Unlike business failures these codes are
 * lowercase snake_case, so the generic upper-case extractor cannot see them.
 */
export function nativeFailureCode(error: unknown, fallback: string): string {
  if (error !== null && typeof error === 'object') {
    const value = (error as Record<string, unknown>).code;
    if (typeof value === 'string' && NATIVE_CODE.test(value)) return value;
  }
  return fallback;
}

/**
 * The accepted snapshot's settings-read failure, or null while the last read succeeded. A
 * damaged local settings file keeps the last known revision and level; this reports the read
 * failure itself and is not a save result.
 */
export function settingsReadErrorText(
  status: DiagnosticsStatus | null,
  language: 'zh' | 'en',
): string | null {
  const code = status?.settings.error;
  if (!code) return null;
  const reason = fixedFailureText(code, language);
  return language === 'zh'
    ? `本机设置读取错误：${reason}`
    : `Could not read the local settings: ${reason}`;
}

/** Opening the local log directory either succeeded or reports one fixed code. */
export function openLogsState(ok: boolean, code: string | undefined, language: 'zh' | 'en'): LogsState {
  if (ok) {
    return {
      phase: 'opened',
      code: language === 'zh' ? '已在本机文件管理器中打开日志目录。' : 'Opened the log directory in the file manager.',
    };
  }
  return { phase: 'failed', code: fixedFailureText(code ?? '', language) };
}

/** One in-flight request at a time, and only while the page is actually visible. */
export function shouldRequest(active: boolean, inFlight: boolean): boolean {
  return active && !inFlight;
}
