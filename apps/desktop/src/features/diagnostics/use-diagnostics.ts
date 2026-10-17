import { hostCapabilities } from '../../host/index.ts';
import { invoke } from '../../host/index.ts';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  acceptResponse,
  nativeFailureCode,
  openLogsState,
  type LogsState,
} from './diagnostics-state';
import {
  type DiagnosticLevel,
  type DiagnosticsStatus,
  DIAGNOSTICS_POLL_MS,
} from './types';

type DiagnosticsHook = {
  status: DiagnosticsStatus | null;
  draft: DiagnosticLevel;
  saving: boolean;
  saveError: string | null;
  logsState: LogsState;
  setDraft: (level: DiagnosticLevel) => void;
  save: () => Promise<void>;
  refresh: () => Promise<void>;
  openLogs: () => Promise<void>;
};

/**
 * The diagnostics view is independent of every business snapshot: it appears and stays
 * usable while the local service is still starting, unreachable or failed. At most one
 * request is in flight, polling stops when the page is not active, and a late response can
 * never overwrite a newer revision or the user's unsaved draft.
 */
export function useDiagnostics(active: boolean, language: 'zh' | 'en'): DiagnosticsHook {
  const [status, setStatus] = useState<DiagnosticsStatus | null>(null);
  const [draft, setDraftState] = useState<DiagnosticLevel>('info');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [logsState, setLogsState] = useState<LogsState>({ phase: 'idle' });
  const inFlight = useRef(false);
  const activeRef = useRef(active);
  const dirty = useRef(false);
  const draftRef = useRef<DiagnosticLevel>('info');
  const statusRef = useRef<DiagnosticsStatus | null>(null);
  const logsInFlight = useRef(false);
  activeRef.current = active;

  const applyStatus = useCallback((next: DiagnosticsStatus) => {
    // The draft and the visible status both come from the accepted snapshot, so a late
    // response with an older revision moves neither of them.
    const applied = acceptResponse(
      statusRef.current,
      next,
      dirty.current ? draftRef.current : null,
    );
    statusRef.current = applied.status;
    setStatus(applied.status);
    setDraftState(applied.draft);
  }, []);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const next = await invoke<DiagnosticsStatus>('diagnostic_status');
      if (activeRef.current) applyStatus(next);
    } catch {
      // A denied or failed read shows nothing rather than a stale success.
    } finally {
      inFlight.current = false;
    }
  }, [applyStatus]);

  useEffect(() => {
    if (!active) {
      setLogsState({ phase: 'idle' });
      return;
    }
    void refresh();
    const timer = window.setInterval(() => {
      if (activeRef.current) void refresh();
    }, DIAGNOSTICS_POLL_MS);
    return () => window.clearInterval(timer);
  }, [active, refresh]);

  const setDraft = useCallback((level: DiagnosticLevel) => {
    dirty.current = true;
    draftRef.current = level;
    setSaveError(null);
    setDraftState(level);
  }, []);

  const save = useCallback(async () => {
    const current = statusRef.current;
    if (!current || saving) return;
    setSaving(true);
    setSaveError(null);
    try {
      const next = await invoke<DiagnosticsStatus>('set_diagnostic_level', {
        input: { expected_revision: current.settings.revision, level: draft },
      });
      dirty.current = false;
      applyStatus(next);
    } catch (error) {
      // The draft stays; the page shows the fixed code and offers a refresh instead of
      // retrying or overwriting a conflict by itself.
      setSaveError(nativeFailureCode(error, 'diagnostics_unavailable'));
    } finally {
      setSaving(false);
    }
  }, [applyStatus, draft, saving]);

  const openLogs = useCallback(async () => {
    if (logsInFlight.current) return;
    logsInFlight.current = true;
    setLogsState({ phase: 'busy' });
    try {
      await invoke<null>('open_diagnostic_directory');
      setLogsState(hostCapabilities().directory_reveal === 'path'
        ? { phase: 'opened', code: language === 'zh' ? '目录路径已显示。' : 'The directory path is displayed.' }
        : openLogsState(true, undefined, language));
    } catch (error) {
      setLogsState(openLogsState(false, nativeFailureCode(error, 'diagnostics_unavailable'), language));
    } finally {
      logsInFlight.current = false;
    }
  }, [language]);

  return {
    status,
    draft,
    saving,
    saveError,
    logsState,
    setDraft,
    save,
    refresh,
    openLogs,
  };
}
