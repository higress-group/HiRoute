import { useCallback, useEffect, useRef, useState } from 'react';
import { safeDiagnosticCode } from '../error-code';
import { invoke } from '../host/index.ts';

type UpdateView = {
  current_version: string;
  phase: 'idle' | 'checking' | 'check_failed' | 'current' | 'available' | 'downloading' | 'verifying' | 'ready' | 'waiting' | 'installing' | 'failed';
  available: { version: string; notes: { zh: string; en: string }; size: number } | null;
  downloaded_bytes: number;
  active_calls: number;
  active_tasks: number;
  error: string | null;
  diagnostic?: { step: string; io_kind: string | null; windows_error: number | null; hresult: number | null; process_exit: number | null } | null;
  can_install: boolean;
  package_ready: boolean;
  recovery: 'service_available' | 'stop_pending' | 'restart_required' | null;
};

type UpdateCommand = 'update_check' | 'update_download' | 'update_install' | 'update_cancel';
const busyPhases = ['checking', 'downloading', 'verifying', 'waiting', 'installing'];
const startupDelayMs = 1000;
const pollIntervalMs = 1000;

export function useDesktopUpdates(settingsActive: boolean, onServiceChanged: () => Promise<void>) {
  const [view, setView] = useState<UpdateView | null>(null);
  const [statusError, setStatusError] = useState('');
  const [actionError, setActionError] = useState('');
  const [actionPending, setActionPending] = useState(false);
  const [cancelPending, setCancelPending] = useState(false);
  const pending = useRef({ action: false, cancel: false });
  const startupCheck = useRef(true);
  const commandRevision = useRef(0);
  const busy = view !== null && busyPhases.includes(view.phase);
  const recovery = view?.recovery;
  const observing = settingsActive || busy || actionPending || cancelPending;

  useEffect(() => {
    if (recovery) void onServiceChanged();
  }, [recovery, onServiceChanged]);

  const act = useCallback(async (command: UpdateCommand) => {
    const key = command === 'update_cancel' ? 'cancel' : 'action';
    if (pending.current[key] || (key === 'action' && pending.current.cancel)) return;
    startupCheck.current = false;
    commandRevision.current += 1;
    pending.current[key] = true;
    const setPending = key === 'cancel' ? setCancelPending : setActionPending;
    setPending(true);
    setActionError('');
    try {
      setView(await invoke<UpdateView>(command));
      setStatusError('');
    } catch (failure) {
      setActionError(safeDiagnosticCode(failure, 'UPGRADE_UNAVAILABLE'));
    } finally {
      commandRevision.current += 1;
      pending.current[key] = false;
      setPending(false);
    }
  }, []);

  useEffect(() => {
    if (!observing && !startupCheck.current) return;
    let current = true;
    let timer: number | undefined;
    async function read() {
      const revision = commandRevision.current;
      try {
        const value = await invoke<UpdateView>('update_status');
        if (!current || commandRevision.current !== revision) return;
        setView(value);
        setStatusError('');
        if (startupCheck.current) {
          startupCheck.current = false;
          // The native host can already have an update or a staged package after a UI reconnect.
          if (value.phase === 'idle' && value.can_install) void act('update_check');
        }
      } catch (failure) {
        if (current && commandRevision.current === revision) {
          setStatusError(safeDiagnosticCode(failure, 'UPGRADE_STATUS_UNAVAILABLE'));
        }
      } finally {
        if (current && observing) timer = window.setTimeout(read, pollIntervalMs);
      }
    }
    if (observing) void read();
    else timer = window.setTimeout(read, startupDelayMs);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [act, observing]);

  return { view, statusError, actionError, busy, actionPending, cancelPending, act };
}
