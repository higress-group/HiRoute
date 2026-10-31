import { buildSaveChange, checkMatches, clientIdempotencyKey, clientOperationId, explicitApiModelSelection, saveEligibility } from '../model-connections/state.ts';
import { networkEdit, type NativeNetworkDraft, type NativeNetworkFailure } from '../model-connections/native-network.ts';
import type { ComputeCandidateRef, ModelConnectionBackend, OperationReference } from '../model-connections/types';
import type { ManagedSource, ManagementSnapshot } from '../models/types';
import { associatedApiOption, subscriptionProvider, type LinkedSubscriptionProvider } from './api-sync.ts';
import { observeSaveAttempt, type SaveAttempt, type SaveAttemptPorts } from './save-attempt.ts';

export const NEW_SUBSCRIPTION_API = '@new-provider-api';
export type InlineApiResult = {
  state: 'saved' | 'pending' | 'needs_input' | 'failed';
  reason?: 'key_required' | 'unavailable' | 'check_failed' | 'models_unavailable' | 'save_failed' | 'link_failed' | 'source_changed' | 'readback_failed';
  api_source_id?: string;
  missing_model_ids?: string[];
  failure?: NativeNetworkFailure | null;
};
export type NewApiPorts = {
  backend: ModelConnectionBackend;
  read(): Promise<ManagementSnapshot>;
  find(key: string): ReturnType<SaveAttemptPorts['find']>;
  associate(sourceId: string, apiSourceId: string, expectedRevision: number, current: () => boolean): Promise<'saved' | 'uncertain'>;
  onOperation(operation: OperationReference): void;
  onUncertain(): void;
  current(): boolean;
};

/** A short-lived credential owner for the inline editor, using normal native authority. */
export class NewSubscriptionApiSetup {
  private provider: LinkedSubscriptionProvider;
  private ports: NewApiPorts;
  private input: ComputeCandidateRef | null = null;
  private attempt: SaveAttempt | null = null;
  private apiSourceId: string | null = null;
  private apiRevision: number | undefined;
  private linkUncertain = false;
  private associationConfirmed = false;
  private owner: { sourceId: string; revision: number } | null = null;
  private checkId: string | null = null;
  private generation = 0;
  private lineage = clientOperationId('subscription-api');
  private running: Promise<InlineApiResult> | null = null;

  constructor(provider: LinkedSubscriptionProvider, ports: NewApiPorts) { this.provider = provider; this.ports = ports; }
  get hasCredential() { return this.input !== null; }
  get awaitingSave() { return Boolean(this.attempt?.submitted && !this.attempt.terminal); }
  get hasSavedApi() { return this.apiSourceId !== null; }

  async dispose() {
    this.generation += 1;
    if (this.attempt?.submitted) return; // Accepted/unknown writes own their input and Check.
    const checkId = this.checkId, input = this.input;
    this.checkId = null;
    this.input = null;
    await Promise.allSettled([
      ...(checkId ? [this.ports.backend.cancelModelConnectionCheck(checkId)] : []),
      ...(input ? [this.ports.backend.releaseProtectedInput(input)] : []),
    ]);
  }

  connect(source: Pick<ManagedSource, 'source_id' | 'revision'>, secret: string, network: NativeNetworkDraft, clearSecret: () => void): Promise<InlineApiResult> {
    if (this.running) return this.running;
    this.running = this.run(source, secret, network, clearSecret).finally(() => { this.running = null; });
    return this.running;
  }

  private completion(snapshot: ManagementSnapshot, source: ManagedSource | undefined): InlineApiResult {
    const api = snapshot.sources.find(item => item.source_id === this.apiSourceId);
    if (!source || subscriptionProvider(source) !== this.provider || !api || api.provenance !== 'registered' || api.state !== 'ready'
      || api.connection_identity?.connection_option_id !== associatedApiOption(this.provider)
      || !snapshot.subscription_api_links?.some(item => item.source_id === source.source_id && item.api_source_id === api.source_id)) {
      return { state: 'failed', reason: 'source_changed' };
    }
    const imported = new Set(api.models.map(model => model.upstream_model_id));
    return { state: 'saved', api_source_id: api.source_id, missing_model_ids: source.models.map(model => model.upstream_model_id).filter(id => !imported.has(id)) };
  }

  private async run(source: Pick<ManagedSource, 'source_id' | 'revision'>, secret: string, network: NativeNetworkDraft, clearSecret: () => void): Promise<InlineApiResult> {
    const generation = this.generation;
    const current = () => this.ports.current() && generation === this.generation;
    const failed = (reason: InlineApiResult['reason'], extra: Partial<InlineApiResult> = {}): InlineApiResult => ({ state: 'failed', reason, ...extra });
    try {
      const snapshot = await this.ports.read();
      const saved = snapshot.sources.find(item => item.source_id === source.source_id);
      if (!saved || subscriptionProvider(saved) !== this.provider) return failed('source_changed');
      if (!this.owner) this.owner = { sourceId: saved.source_id, revision: source.revision };
      if (this.owner.sourceId !== saved.source_id) return failed('source_changed');
      const link = snapshot.subscription_api_links?.find(item => item.source_id === saved.source_id);
      if (saved.revision !== this.owner.revision && (!this.apiSourceId || link?.api_source_id !== this.apiSourceId)) return failed('source_changed');
      if (!current()) return { state: 'pending' };
      // Association and its synchronization are durable. Retrying completion only reads;
      // their own API revisions must not be compared with the pre-association creation.
      if (this.associationConfirmed) return this.completion(snapshot, saved);

      if (!this.apiSourceId && !this.attempt) {
        if (secret.trim()) {
          if (this.input) await this.ports.backend.releaseProtectedInput(this.input);
          this.input = null;
          const registered = await this.ports.backend.registerProtectedInput(secret);
          if (!current()) {
            await this.ports.backend.releaseProtectedInput(registered.input_candidate).catch(() => {});
            return { state: 'pending' };
          }
          clearSecret();
          this.input = registered.input_candidate;
        }
        if (!this.input) return { state: 'needs_input', reason: 'key_required' };
        const options = await this.ports.backend.listConnectionOptions();
        const optionId = associatedApiOption(this.provider);
        const option = options.options.find(item => item.connection_option_id === optionId && item.registered_check_available && item.origin === 'native_api');
        if (!option) return failed('unavailable');
        if (!current()) return { state: 'pending' };
        const baseName = this.provider === 'codex' ? 'OpenAI API' : 'Anthropic API';
        const names = new Set(snapshot.sources.map(item => item.display_name));
        let name = baseName;
        for (let i = 2; names.has(name); i += 1) name = `${baseName} ${i}`;
        const checkId = clientOperationId('subscription-api-check');
        this.checkId = checkId;
        const checked = await this.ports.backend.checkRegisteredModelConnection({
          display_name: name, connection_option_id: optionId, expected_catalog: options.catalog,
          candidate_ref: null, lineage_ref: this.lineage, edit_revision: 1, check_id: checkId,
          inference_model_id: null, models: [], input_candidate: this.input, existing_source_id: null,
        }, networkEdit(network));
        if (!current()) return { state: 'pending' };
        if (!checkMatches(checked, { candidateRef: null, editRevision: 1, checkId })
          || checked.candidate.producer !== 'native' || checked.candidate.provenance !== 'registered' || checked.candidate.existing_source_id
          || !['verified', 'not_required'].includes(checked.authentication)
          || !['available', 'empty', 'partial'].includes(checked.directory)
          || (checked.failure && checked.failure.phase !== 'directory')) {
          return failed('check_failed', { failure: checked.failure });
        }
        const refs = [...explicitApiModelSelection(checked, saved.models.map(model => model.upstream_model_id))];
        if (!refs.length) return failed('models_unavailable');
        if (!saveEligibility(checked, true).allowed) return failed('check_failed');
        const fresh = await this.ports.read();
        if (fresh.sources.find(item => item.source_id === saved.source_id)?.revision !== this.owner.revision) return failed('source_changed');
        if (!current()) return { state: 'pending' };
        const change = buildSaveChange({ result: checked, expectedRevisions: fresh.revisions, selectedModelRefs: refs, enable: true, protectedInput: this.input });
        const preview = await this.ports.backend.previewComputeSave(change, networkEdit(network));
        if (!current()) return { state: 'pending' };
        this.attempt = {
          request: { spec: preview.spec, accept_digest: preview.accept_digest, expected_revisions: preview.expected_revisions, idempotency_key: clientIdempotencyKey('subscription-api-create') },
          submitted: false,
        };
      }
      if (!this.apiSourceId) {
        const result = await observeSaveAttempt(this.attempt!, {
          apply: request => this.ports.backend.applyComputeSave(request),
          result: operation => this.ports.backend.getComputeSaveResult(operation),
          find: key => this.ports.find(key), onOperation: this.ports.onOperation, onUncertain: this.ports.onUncertain, current,
        });
        if (result.disposition === 'pending') return { state: 'pending' };
        this.input = null; // A completed native Operation has consumed/released its protected input.
        if (result.disposition !== 'saved' || !result.source_id) {
          this.attempt = null;
          return failed('save_failed');
        }
        this.apiSourceId = result.source_id;
        this.apiRevision = result.saved_revision;
      }
      if (!current()) return { state: 'pending', api_source_id: this.apiSourceId };
      const after = await this.ports.read();
      const api = after.sources.find(item => item.source_id === this.apiSourceId);
      if (!api || api.provenance !== 'registered' || api.state !== 'ready'
        || api.connection_identity?.connection_option_id !== associatedApiOption(this.provider)
        || (this.apiRevision !== undefined && api.revision !== this.apiRevision)) return failed('source_changed');
      if (!saved.models.some(model => api.models.some(item => item.upstream_model_id === model.upstream_model_id))) return failed('source_changed');
      const currentSource = after.sources.find(item => item.source_id === saved.source_id);
      const alreadyLinked = after.subscription_api_links?.some(item => item.source_id === saved.source_id && item.api_source_id === api.source_id);
      if (!currentSource || (!alreadyLinked && currentSource.revision !== this.owner.revision)) return failed('source_changed');
      if (!alreadyLinked || this.linkUncertain) {
        if (!current()) return { state: 'pending', api_source_id: api.source_id };
        this.linkUncertain = true;
        const linked = await this.ports.associate(saved.source_id, api.source_id, this.owner.revision, current);
        if (linked === 'uncertain') return { state: 'pending', api_source_id: api.source_id };
        this.linkUncertain = false;
      }
      this.associationConfirmed = true;
      const completed = await this.ports.read();
      return this.completion(completed, completed.sources.find(item => item.source_id === saved.source_id));
    } catch {
      if (this.attempt && !this.attempt.submitted) this.attempt = null;
      if (this.associationConfirmed) return { state: 'pending', reason: 'readback_failed', api_source_id: this.apiSourceId! };
      return failed(this.apiSourceId ? 'link_failed' : 'check_failed');
    }
  }
}
