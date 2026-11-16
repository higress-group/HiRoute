import { safeDiagnosticCode } from '../../error-code';
import { invoke } from '../../host/index.ts';
import type { TrialSetupSnapshot } from './types';

export async function readTrialSetup(): Promise<TrialSetupSnapshot> {
  for (let attempt = 0; ; attempt += 1) {
    try { return await invoke<TrialSetupSnapshot>('trial_setup_snapshot'); }
    catch (error) {
      const code = safeDiagnosticCode(error, 'UNKNOWN');
      if (attempt >= 1 || !['READ_SNAPSHOT_CHANGED', 'CHANGE_PREVIEW_STALE', 'application.error.change_preview_stale'].includes(code)) throw error;
    }
  }
}
