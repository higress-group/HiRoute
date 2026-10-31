import { flattenNetworkCheck, networkEdit, type NetworkCheckResponse } from './native-network';
import { invoke } from '../../host/index.ts';
import type { ModelConnectionBackend } from './types';
import type { ManagementSnapshot } from '../models/types';

/** The same protected Check / Preview / Apply owner serves management and trial setup. */
export function modelConnectionBackend(language: 'zh' | 'en'): ModelConnectionBackend {
  return {
    registerProtectedInput: secret => invoke('register_protected_model_input', { secret }),
    releaseProtectedInput: input => invoke('release_protected_model_input', { input }),
    listConnectionOptions: () => invoke('compute_connection_options'),
    checkModelConnection: async (request, edit = networkEdit()) => flattenNetworkCheck(await invoke<NetworkCheckResponse>('check_model_connection_v2', { request: { request, network_edit: edit } })),
    checkRegisteredModelConnection: async (request, edit = networkEdit()) => flattenNetworkCheck(await invoke<NetworkCheckResponse>('check_registered_model_connection_v2', { request: { request, network_edit: edit } })),
    cancelModelConnectionCheck: checkId => invoke('cancel_model_connection_check', { checkId }),
    previewComputeSave: async (change, edit) => {
      const current = await invoke<ManagementSnapshot>('compute_management_snapshot');
      if (edit) return invoke('preview_compute_save_v4', { change: { ...change, expected_revisions: current.revisions }, defaultPreferences: {}, networkEdit: edit });
      return invoke('preview_compute_save_v3', { change: { ...change, expected_revisions: current.revisions }, defaultPreferences: {} });
    },
    applyComputeSave: request => invoke('apply_compute_save', { request, language }),
    getComputeSaveResult: operation => invoke('get_compute_save_result', { operation }),
  };
}

export async function checkSavedNativeConnection(request: import('./types').SavedModelConnectionCheckRequest) {
  return flattenNetworkCheck(await invoke<NetworkCheckResponse>('check_saved_model_connection_v2', { request: { request, network_edit: null } }));
}
