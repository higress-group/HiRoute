import { invoke } from '../../host/index.ts';
import type { ModelConnectionBackend } from './types';
import type { ManagementSnapshot } from '../models/types';

/** The same protected Check / Preview / Apply owner serves management and trial setup. */
export function modelConnectionBackend(language: 'zh' | 'en'): ModelConnectionBackend {
  return {
    registerProtectedInput: secret => invoke('register_protected_model_input', { secret }),
    releaseProtectedInput: input => invoke('release_protected_model_input', { input }),
    listConnectionOptions: () => invoke('compute_connection_options'),
    checkModelConnection: request => invoke('check_model_connection', { request }),
    checkRegisteredModelConnection: request => invoke('check_registered_model_connection', { request }),
    cancelModelConnectionCheck: checkId => invoke('cancel_model_connection_check', { checkId }),
    previewComputeSave: async change => {
      const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
      return invoke('preview_compute_save_v3', { change: { ...change, expected_revisions: current.revisions }, defaultPreferences: {} });
    },
    applyComputeSave: request => invoke('apply_compute_save', { request, language }),
    getComputeSaveResult: operation => invoke('get_compute_save_result', { operation }),
  };
}
