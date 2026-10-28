import type { ComputeCandidateRef, ModelConnectionCheckView, ModelConnectionDraft, ModelDeclaration } from './types';

export type ConnectionSelection = { result: ModelConnectionCheckView; refs: Set<string> };
export type ConnectionEditingDraft = {
  draft: ModelConnectionDraft;
  custom: boolean;
  metadataPrefillKey: string;
  secret: string;
  protectedInput: ComputeCandidateRef | null;
  credentialReentry: boolean;
  selection: ConnectionSelection | null;
};

// Only user-editable facts belong in this comparison. In particular, checks and
// generated IDs/revisions do not turn a visit into an edit. Capability provenance
// can change during editing even when the user restores the original value.
function editableDraft(draft: ModelConnectionDraft, placeholders: Map<string, ModelDeclaration> | undefined) {
  const endpoint = (value: ModelConnectionDraft | ModelConnectionDraft['additional_endpoints'][number]) => ({
    base_url: value.base_url, base_kind: value.base_kind,
    request_path_override: value.request_path_override, inventory_path_override: value.inventory_path_override,
    protocol: value.protocol, authentication: value.authentication,
  });
  const models = draft.models.filter(model => {
    const original = placeholders?.get(model.client_id);
    return !original || JSON.stringify(model) !== JSON.stringify(original);
  }).map(model => ({
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
  private placeholders = new Map<string, Map<string, ModelDeclaration>>();
  private prefills = new Map<string, Map<string, ConnectionEditingDraft>>();
  private prefillBaselines = new Map<string, Map<string, ModelConnectionDraft>>();

  get hasDrafts() { return this.drafts.size > 0; }
  begin(key: string, draft: ModelConnectionDraft) {
    if (!this.baselines.has(key)) this.baselines.set(key, structuredClone(draft));
  }
  beginPrefill(key: string | null, prefill: string, draft: ModelConnectionDraft) {
    if (!key) return;
    const values = this.prefillBaselines.get(key) ?? new Map<string, ModelConnectionDraft>();
    if (!values.has(prefill)) values.set(prefill, structuredClone(draft));
    this.prefillBaselines.set(key, values);
  }
  changed(key: string | null, draft: ModelConnectionDraft) {
    const baseline = key ? this.baselines.get(key) : undefined;
    const placeholders = key ? this.placeholders.get(key) : undefined;
    return baseline ? JSON.stringify(editableDraft(draft, placeholders)) !== JSON.stringify(editableDraft(baseline, placeholders)) : false;
  }
  recordAutoPlaceholder(key: string | null, model: ModelDeclaration) {
    if (!key || model.upstream_model_id !== '' || model.display_name !== '') return;
    const placeholders = this.placeholders.get(key) ?? new Map<string, ModelDeclaration>();
    placeholders.set(model.client_id, structuredClone(model));
    this.placeholders.set(key, placeholders);
  }
  applyModelDefaults(key: string | null, update: (models: ModelDeclaration[]) => ModelDeclaration[], prefill?: string) {
    const baseline = key ? this.baselines.get(key) : undefined;
    if (baseline && (prefill === undefined || prefill === '')) baseline.models = update(baseline.models);
    const prefillBaseline = key && prefill !== undefined ? this.prefillBaselines.get(key)?.get(prefill) : undefined;
    if (prefillBaseline) prefillBaseline.models = update(prefillBaseline.models);
  }
  get hasUnsavedDrafts() {
    const changed = (key: string, value: ConnectionEditingDraft) => this.changed(key, value.draft)
      || Boolean(value.secret || value.protectedInput || value.credentialReentry || value.selection?.refs.size);
    return Array.from(this.drafts, ([key, value]) => changed(key, value)).some(Boolean)
      || Array.from(this.prefills, ([key, values]) => Array.from(values, ([prefill, value]) => {
        const baseline = this.prefillBaselines.get(key)?.get(prefill);
        const placeholders = this.placeholders.get(key);
        const editableChanged = baseline
          ? JSON.stringify(editableDraft(value.draft, placeholders)) !== JSON.stringify(editableDraft(baseline, placeholders))
          : this.changed(key, value.draft);
        return editableChanged || Boolean(value.secret || value.protectedInput || value.credentialReentry || value.selection?.refs.size);
      }).some(Boolean)).some(Boolean);
  }
  stashPrefill(key: string | null, prefill: string, value: ConnectionEditingDraft) {
    if (!key) return;
    const values = this.prefills.get(key) ?? new Map<string, ConnectionEditingDraft>();
    values.set(prefill, value);
    this.prefills.set(key, values);
  }
  takePrefill(key: string | null, prefill: string) {
    const values = key ? this.prefills.get(key) : undefined;
    const value = values?.get(prefill);
    values?.delete(prefill);
    return value;
  }
  stash(key: string, value: ConnectionEditingDraft) { this.drafts.set(key, value); }
  take(key: string) {
    const value = this.drafts.get(key);
    this.drafts.delete(key);
    return value;
  }
  clear(release: (input: ComputeCandidateRef) => void) {
    const clearDraft = (value: ConnectionEditingDraft) => {
      value.secret = '';
      if (value.protectedInput) release(value.protectedInput);
      value.protectedInput = null;
    };
    for (const value of this.drafts.values()) clearDraft(value);
    for (const values of this.prefills.values()) for (const value of values.values()) clearDraft(value);
    this.drafts.clear();
    this.baselines.clear();
    this.placeholders.clear();
    this.prefills.clear();
    this.prefillBaselines.clear();
  }
}
