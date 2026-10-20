import { randomId } from '../../random-id.ts';
import type { ModelConnectionDraft } from './types';

export function newModelConnectionDraft(kind: 'known' | 'custom' | 'free' = 'custom'): ModelConnectionDraft {
  const known = kind === 'known';
  return {
    entry_kind: kind === 'free' ? 'free_api_key' : known ? 'preset_api' : 'custom_api',
    candidate_ref: null,
    lineage_ref: `lineage/native/${randomId()}`,
    display_name: '',
    existing_source_id: null,
    expected_source_revision: null,
    edit_revision: 1,
    check_id: '',
    base_url: '',
    base_kind: 'api_root',
    request_path_override: null,
    inventory_path_override: null,
    protocol: 'chat_completions',
    protocol_profile_id: 'profile/custom/chat_completions',
    protocol_profile_revision: 1,
    authentication: { kind: 'bearer' },
    additional_endpoints: [],
    provenance: { kind: 'user_configured', configuration_revision: 1 },
    qualification: { free_access: null, evidence_ref: null },
    models: [],
  };
}
