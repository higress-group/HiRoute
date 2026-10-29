import type { Options } from '../../plan-editor';
import type { MaterializationState, Provenance, RevisionSet } from '../models/types';
import type { ModelAvailabilityResult } from '../model-feed/types';

/** Safe fields projected with the same routing and catalog capture as options. */
export type TrialMember = {
  model_ref: string; binding_id: string; revision: number; upstream_model_id: string;
  display_name: string; catalog_configuration_id?: string;
  membership: 'catalog' | 'observed' | 'user_declared'; catalog_admitted: boolean;
  availability: 'available' | 'cooling_down' | 'disabled' | 'needs_credentials' | 'unavailable' | 'unknown';
  reason_code?: string;
};
export type TrialSource = {
  source_id: string; revision: number; display_name: string; provenance: Provenance;
  access_kind: 'api' | 'subscription' | 'unknown'; state: MaterializationState;
  models: TrialMember[]; actions: string[];
};
export type TrialRetainedApiConnection = {
  source_id: string; source_revision: number; binding_id: string; binding_revision: number;
  model_configuration_id: string; display_name: string;
};
export type TrialManagementSnapshot = { revisions: RevisionSet; sources: TrialSource[]; retained_api_connections: TrialRetainedApiConnection[] };
export type TrialSetupSnapshot = { schema: 'hiroute.trial-setup-snapshot/v1'; options: Options; management: TrialManagementSnapshot };
export type TrialCatalogSnapshot = { availability: ModelAvailabilityResult; options: Options; management: TrialManagementSnapshot };
