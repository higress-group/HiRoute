import type { ComputeCandidateRef, ModelConnectionCheckView, ModelConnectionDraft, ModelDeclaration } from './types';

export type ConnectionSelection = { result: ModelConnectionCheckView; refs: Set<string> };
export type ConnectionEditingDraft = {
  draft: ModelConnectionDraft;
  custom: boolean;
  metadataPrefillKey: string;
  secret: string;
  protectedInput: ComputeCandidateRef | null;
  selection: ConnectionSelection | null;
};

// Only user-editable facts belong in this comparison. In particular, checks and
// generated IDs/revisions do not turn a visit into an edit. Capability provenance
// can change during editing even when the user restores the original value.
function editableDraft(draft: ModelConnectionDraft) {
  const endpoint = (value: ModelConnectionDraft | ModelConnectionDraft['additional_endpoints'][number]) => ({
    base_url: value.base_url, base_kind: value.base_kind,
    request_path_override: value.request_path_override, inventory_path_override: value.inventory_path_override,
    protocol: value.protocol, authentication: value.authentication,
  });
  const models = draft.models.map(model => ({
    upstream_model_id: model.upstream_model_id, display_name: model.display_name,
    capabilities: Object.fromEntries(Object.entries(model.capabilities).map(([key, fact]) => [key, fact.value])),
  })).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  return { display_name: draft.display_name, display_template_id: draft.display_template_id ?? null,
    ...endpoint(draft), additional_endpoints: draft.additional_endpoints.map(endpoint), models };
}

/** Credentials live only in this dialog's memory; never in a serializable UI draft. */
export class ConnectionEditingSession {
  private drafts = new Map<string, ConnectionEditingDraft>();
  private baselines = new Map<string, ModelConnectionDraft>();

  get hasDrafts() { return this.drafts.size > 0; }
  begin(key: string, draft: ModelConnectionDraft) {
    if (!this.baselines.has(key)) this.baselines.set(key, structuredClone(draft));
  }
  changed(key: string | null, draft: ModelConnectionDraft) {
    const baseline = key ? this.baselines.get(key) : undefined;
    return baseline ? JSON.stringify(editableDraft(draft)) !== JSON.stringify(editableDraft(baseline)) : false;
  }
  applyModelDefaults(key: string | null, update: (models: ModelDeclaration[]) => ModelDeclaration[]) {
    const baseline = key ? this.baselines.get(key) : undefined;
    if (baseline) baseline.models = update(baseline.models);
  }
  get hasUnsavedDrafts() {
    return Array.from(this.drafts, ([key, value]) => this.changed(key, value.draft)
      || Boolean(value.secret || value.protectedInput || value.selection?.refs.size)).some(Boolean);
  }
  stash(key: string, value: ConnectionEditingDraft) { this.drafts.set(key, value); }
  take(key: string) {
    const value = this.drafts.get(key);
    this.drafts.delete(key);
    return value;
  }
  clear(release: (input: ComputeCandidateRef) => void) {
    for (const value of this.drafts.values()) {
      value.secret = '';
      if (value.protectedInput) release(value.protectedInput);
      value.protectedInput = null;
    }
    this.drafts.clear();
    this.baselines.clear();
  }
}
