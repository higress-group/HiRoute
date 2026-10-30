import type {
  DecisionAuthHeader, DecisionExpectedSourceRevision, DecisionServiceRecord, DecisionServiceTestResult, DecisionServiceV2,
  DecisionServiceView, DecisionSourceSummary, DecisionWireProtocol,
} from './contracts.ts';
import { providers } from './types.ts';

/** Native decision-model authoring state. Plaintext Keys never enter this state. */
export type DecisionMode = 'shared' | 'independent';
export type DecisionBundle = { model: string; endpoint: string; wire_protocol: DecisionWireProtocol | null };
export type IndependentPreset = { id: string; endpoint: string; model: string; wire_protocol: DecisionWireProtocol | null };
/** A failed refresh keeps the last successful list only as unavailable state, never as missing. */
export type SourceRead = { status: 'loading'; previous?: DecisionSourceSummary[] } | { status: 'ready'; sources: DecisionSourceSummary[] } | { status: 'error'; code?: string; previous?: DecisionSourceSummary[] };
export function knownSources(read: SourceRead): DecisionSourceSummary[] {
  return read.status === 'ready' ? read.sources : (read.previous ?? []).map(source => ({ ...source, state: 'unavailable' as const }));
}
export type NativeDraft = {
  id: string;
  /** Null only while a new form awaits its first source read. */
  mode: DecisionMode | null;
  /** Explicit or saved intent. A late source read never changes it. */
  modeChosen: boolean;
  sourceId: string | null;
  provider: string;
  /** One non-secret model/URL/codec bundle per mode/source context. */
  bundles: Record<string, DecisionBundle>;
  name: string; nameEdited: boolean; timeout_ms: number;
  savedIndependent: { provider: string; endpoint: string; auth_header: DecisionAuthHeader } | null;
  savedShared: { source_id: string; lineage_digest: string } | null;
};

export const independentPresets: IndependentPreset[] = providers.map(preset => {
  const protocol = preset.wire_protocol;
  const wire_protocol: DecisionWireProtocol | null = protocol === 'openai_decisions' || protocol === 'system_one'
    ? protocol : protocol === undefined && preset.id !== 'compatible' ? 'system_one' : null;
  return { id: preset.id, endpoint: preset.endpoint, model: preset.model, wire_protocol };
});

export function presetName(id: string, language: 'zh' | 'en'): string {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  switch (id) {
    case 'bailian-token-plan': return t('百炼 Token Plan', 'Bailian Token Plan');
    case 'bailian-workspace': return t('百炼业务空间', 'Bailian workspace');
    case 'openai': return 'OpenAI';
    case 'openrouter': return 'OpenRouter';
    case 'typesafe': return 'TypeSafe';
    case 'compatible': return t('兼容接入', 'Compatible connection');
    default: return id;
  }
}

export function isNativeRecord(record: DecisionServiceRecord): boolean {
  return record.connection.kind === 'system_one' || record.connection.kind === 'native';
}
export function isV2(record: DecisionServiceRecord): record is DecisionServiceV2 { return 'schema' in record; }
export function recordModel(record: DecisionServiceRecord): string | null {
  return record.connection.kind === 'system_one' || record.connection.kind === 'native' ? record.connection.model : null;
}
export function recordProtocol(record: DecisionServiceRecord): DecisionWireProtocol | null {
  return record.connection.kind === 'native' ? record.connection.wire_protocol : record.connection.kind === 'system_one' ? 'system_one' : null;
}
export function recordSharedSource(record: DecisionServiceRecord): string | null {
  return record.connection.kind === 'native' && record.connection.authentication.kind === 'shared_source' ? record.connection.authentication.source_id : null;
}
export function recordProvider(record: DecisionServiceRecord): string | undefined {
  return record.connection.kind === 'system_one' || record.connection.kind === 'native' ? record.connection.provider : undefined;
}

export function contextKey(draft: Pick<NativeDraft, 'mode' | 'sourceId' | 'provider'>): string | null {
  if (draft.mode === 'shared') return draft.sourceId ? `shared:${draft.sourceId}` : null;
  return draft.mode === 'independent' ? `independent:${draft.provider}` : null;
}

export function sourceFor(draft: NativeDraft, read: SourceRead, view?: DecisionServiceView | null): DecisionSourceSummary | null {
  if (!draft.sourceId) return null;
  const listed = knownSources(read).find(source => source.source_id === draft.sourceId);
  if (listed) return listed;
  // A saved source stays identifiable when the list omits it.
  const related = view?.source?.source_id === draft.sourceId ? view.source : view?.related_sources.find(source => source.source_id === draft.sourceId);
  if (read.status === 'ready') return related ? { ...related, state: related.state === 'unavailable' ? 'unavailable' : 'missing' }
    : { source_id: draft.sourceId, lineage_digest: null, revision: null, display_name: null, provider_label: null, endpoint_host: null, auth_kind: null, state: 'missing', credential_state_revision: null, suggestion: null };
  return { ...(related ?? { source_id: draft.sourceId, lineage_digest: null, revision: null, display_name: null, provider_label: null, endpoint_host: null, auth_kind: null, credential_state_revision: null, suggestion: null }), state: 'unavailable' };
}

export function suggestedBundle(draft: NativeDraft, source: DecisionSourceSummary | null): DecisionBundle {
  if (draft.mode === 'shared') {
    return { model: source?.suggestion?.model ?? '', endpoint: source?.suggestion?.endpoint ?? '', wire_protocol: source?.suggestion?.wire_protocol ?? null };
  }
  const preset = independentPresets.find(item => item.id === draft.provider);
  return { model: preset?.model ?? '', endpoint: preset?.endpoint ?? '', wire_protocol: preset?.wire_protocol ?? null };
}

export function currentBundle(draft: NativeDraft, source: DecisionSourceSummary | null): DecisionBundle | null {
  const key = contextKey(draft);
  return key ? draft.bundles[key] ?? suggestedBundle(draft, source) : null;
}

function materialize(draft: NativeDraft, source: DecisionSourceSummary | null): NativeDraft {
  const key = contextKey(draft);
  if (!key || draft.bundles[key]) return draft;
  return { ...draft, bundles: { ...draft.bundles, [key]: suggestedBundle(draft, source) } };
}

export function newNativeDraft(id: string, entry: 'system_one' | 'compatible' = 'system_one'): NativeDraft {
  const compatible = entry === 'compatible';
  const draft: NativeDraft = {
    id, mode: compatible ? 'independent' : null, modeChosen: compatible, sourceId: null,
    provider: compatible ? 'compatible' : independentPresets[0].id, bundles: {},
    name: '', nameEdited: false, timeout_ms: 10000, savedIndependent: null, savedShared: null,
  };
  return materialize(draft, null);
}

/** Prepares an editable V2 draft; the record itself is never rewritten here. */
export function draftFromRecord(record: DecisionServiceRecord): NativeDraft {
  const connection = record.connection;
  const base = { id: record.id, modeChosen: true, name: record.name, nameEdited: true, savedIndependent: null, savedShared: null, sourceId: null } as const;
  if (connection.kind === 'system_one') {
    return { ...base, mode: 'independent', provider: connection.provider, timeout_ms: connection.timeout_ms,
      bundles: { [`independent:${connection.provider}`]: { model: connection.model, endpoint: connection.endpoint, wire_protocol: 'system_one' } },
      savedIndependent: { provider: connection.provider, endpoint: connection.endpoint, auth_header: connection.auth_header } };
  }
  if (connection.kind !== 'native') throw new Error('custom extensions use their own form');
  const bundle = { model: connection.model, endpoint: connection.endpoint, wire_protocol: connection.wire_protocol };
  if (connection.authentication.kind === 'shared_source') {
    const sourceId = connection.authentication.source_id;
    return { ...base, mode: 'shared', sourceId, provider: independentPresets[0].id, timeout_ms: connection.timeout_ms,
      bundles: { [`shared:${sourceId}`]: bundle },
      savedShared: { source_id: sourceId, lineage_digest: connection.authentication.source_lineage_digest } };
  }
  return { ...base, mode: 'independent', provider: connection.provider, timeout_ms: connection.timeout_ms,
    bundles: { [`independent:${connection.provider}`]: bundle },
    savedIndependent: { provider: connection.provider, endpoint: connection.endpoint, auth_header: connection.authentication.auth_header } };
}

/** Applies a successful first read as a default only; it never selects an account. */
export function applySourceDefault(draft: NativeDraft, sources: DecisionSourceSummary[]): NativeDraft {
  if (draft.modeChosen || draft.mode !== null) return draft;
  const mode: DecisionMode = sources.some(source => source.state === 'ready') ? 'shared' : 'independent';
  return materialize({ ...draft, mode }, null);
}

export function chooseMode(draft: NativeDraft, mode: DecisionMode, source: DecisionSourceSummary | null): NativeDraft {
  return materialize({ ...draft, mode, modeChosen: true }, mode === 'shared' ? source : null);
}
/** An explicit choice accepts the source's current lineage. */
export function chooseSource(draft: NativeDraft, source: DecisionSourceSummary | null, sourceId: string | null): NativeDraft {
  return materialize({ ...draft, mode: 'shared', modeChosen: true, sourceId, savedShared: null }, source);
}
/** A saved shared binding whose source was replaced needs an explicit new choice. */
export function lineageChanged(draft: NativeDraft, source: DecisionSourceSummary | null): boolean {
  return draft.mode === 'shared' && !!draft.savedShared && draft.savedShared.source_id === draft.sourceId
    && !!source?.lineage_digest && source.lineage_digest !== draft.savedShared.lineage_digest;
}
export function chooseProvider(draft: NativeDraft, provider: string): NativeDraft {
  return materialize({ ...draft, mode: 'independent', modeChosen: true, provider }, null);
}
export function editBundle(draft: NativeDraft, source: DecisionSourceSummary | null, patch: Partial<DecisionBundle>): NativeDraft {
  const key = contextKey(draft), bundle = currentBundle(draft, source);
  return key && bundle ? { ...draft, bundles: { ...draft.bundles, [key]: { ...bundle, ...patch } } } : draft;
}
/** Explicit reset of the current context. A user-authored display name is retained. */
export function resetToSuggested(draft: NativeDraft, source: DecisionSourceSummary | null): NativeDraft {
  const key = contextKey(draft);
  return key ? { ...draft, bundles: { ...draft.bundles, [key]: suggestedBundle(draft, source) } } : draft;
}
export function sameBundle(left: DecisionBundle | null, right: DecisionBundle | null): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function sourceName(source: Pick<DecisionSourceSummary, 'display_name' | 'source_id'> | null, language: 'zh' | 'en'): string {
  if (!source) return '';
  return source.display_name?.trim() || (language === 'zh' ? `API 接入 ${shortId(source.source_id)}` : `API connection ${shortId(source.source_id)}`);
}
export function shortId(id: string): string { return id.slice(-8); }

export function defaultName(draft: NativeDraft, source: DecisionSourceSummary | null, language: 'zh' | 'en'): string {
  const model = currentBundle(draft, source)?.model.trim() ?? '';
  const label = draft.mode === 'shared' ? sourceName(source, language) : draft.mode === 'independent' ? presetName(draft.provider, language) : '';
  return [model, label].filter(Boolean).join(' · ') || (language === 'zh' ? '决策模型' : 'Decision model');
}
export function displayName(draft: NativeDraft, source: DecisionSourceSummary | null, language: 'zh' | 'en'): string {
  return draft.nameEdited ? draft.name : defaultName(draft, source, language);
}

export function sameEndpointOrigin(left: string, right: string): boolean {
  try { return new URL(left).origin === new URL(right).origin; } catch { return false; }
}
export function endpointHost(endpoint: string): string | null {
  // Source summaries expose the hostname, without the port. Origin checks remain separate.
  try { return new URL(endpoint).hostname || null; } catch { return null; }
}

/** The saved protected reference survives only for the same provider and permitted origin. */
export function retainedIndependentAuth(draft: NativeDraft, source: DecisionSourceSummary | null = null): DecisionAuthHeader | null {
  const saved = draft.savedIndependent, bundle = currentBundle(draft, source);
  return draft.mode === 'independent' && saved && bundle && saved.provider === draft.provider && sameEndpointOrigin(bundle.endpoint, saved.endpoint)
    ? saved.auth_header : null;
}

/** Semantic comparison value: an untouched V1 preparation equals its own baseline. */
export function comparable(draft: NativeDraft, source: DecisionSourceSummary | null, language: 'zh' | 'en'): string {
  return JSON.stringify({
    mode: draft.mode, source: draft.mode === 'shared' ? draft.sourceId : null, provider: draft.mode === 'independent' ? draft.provider : null,
    bundle: currentBundle(draft, source), name: displayName(draft, source, language).trim(), timeout: draft.timeout_ms,
    auth: retainedIndependentAuth(draft, source)?.value_secret_ref ?? null,
  });
}

export function sharedProvider(protocol: DecisionWireProtocol): string { return protocol === 'openai_decisions' ? 'openai' : 'compatible'; }

export type NativeIssueKey = 'mode' | 'source' | 'model' | 'endpoint' | 'wire_protocol' | 'name' | 'timeout' | 'credential';
export function nativeIssues(draft: NativeDraft, source: DecisionSourceSummary | null, hasSecret: boolean, replacing: boolean, language: 'zh' | 'en'): Partial<Record<NativeIssueKey, string>> {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const issues: Partial<Record<NativeIssueKey, string>> = {};
  if (!draft.mode) { issues.mode = t('请选择配置方式。', 'Choose how to configure credentials.'); return issues; }
  if (draft.mode === 'shared') {
    if (!draft.sourceId) issues.source = t('请选择 API 接入。', 'Choose an API connection.');
    else if (!source || source.state === 'missing') issues.source = t('这个 API 接入已不存在，请选择其他接入。', 'This API connection no longer exists. Choose another connection.');
    else if (lineageChanged(draft, source)) issues.source = t('这个 API 接入已被替换，请重新选择接入。', 'This API connection was replaced. Choose a connection again.');
    else if (source.state === 'unavailable' || source.revision === null || !source.lineage_digest) issues.source = t('暂时无法读取这个 API 接入的状态，请重试后再保存。', 'This API connection’s state could not be read. Retry before saving.');
  }
  const bundle = currentBundle(draft, source);
  if (bundle) {
    if (!bundle.model.trim()) issues.model = t('请填写模型 ID。', 'Enter a model ID.');
    try {
      const url = new URL(bundle.endpoint);
      if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.hash) throw new Error();
    } catch { issues.endpoint = t('请填写完整 HTTP 或 HTTPS 地址，不含用户名、密码或片段。', 'Enter a complete HTTP or HTTPS URL without user info or a fragment.'); }
    if (!bundle.wire_protocol) issues.wire_protocol = t('请选择这个地址使用的协议。', 'Choose the protocol this address uses.');
  }
  if (!displayName(draft, source, language).trim()) issues.name = t('请填写名称。', 'Enter a name.');
  if (!Number.isInteger(draft.timeout_ms) || draft.timeout_ms < 1 || draft.timeout_ms > 3_600_000) issues.timeout = t('超时需在 0.001 到 3600 秒之间。', 'The timeout must be between 0.001 and 3600 seconds.');
  if (draft.mode === 'independent' && (replacing || !retainedIndependentAuth(draft, source)) && !hasSecret) issues.credential = t('请填写 API Key。', 'Enter an API key.');
  return issues;
}

/** Builds the next immutable V2 revision. Call only after nativeIssues is empty. */
export function nativeService(draft: NativeDraft, source: DecisionSourceSummary | null, revision: number, replacing: boolean, language: 'zh' | 'en'): DecisionServiceV2 {
  const bundle = currentBundle(draft, source)!;
  const wire_protocol = bundle.wire_protocol!;
  const retained = replacing ? null : retainedIndependentAuth(draft, source);
  const authentication = draft.mode === 'shared'
    ? { kind: 'shared_source' as const, source_id: draft.sourceId!, source_lineage_digest: source?.lineage_digest ?? draft.savedShared!.lineage_digest }
    : { kind: 'independent' as const, auth_header: retained ?? { name: 'Authorization', value_secret_ref: '' } };
  return {
    schema: 'hiroute.decision-service/v2', id: draft.id, revision, name: displayName(draft, source, language).trim(),
    connection: {
      kind: 'native', provider: draft.mode === 'shared' ? sharedProvider(wire_protocol) : draft.provider, wire_protocol,
      model: bundle.model.trim(), endpoint: bundle.endpoint.trim(), timeout_ms: draft.timeout_ms, authentication,
    },
  };
}

/** Each affected source appears once, using the freshest observed revision. */
export function expectedSourceRevisions(sourceIds: (string | null | undefined)[], read: SourceRead, view?: DecisionServiceView | null): DecisionExpectedSourceRevision[] {
  const result: DecisionExpectedSourceRevision[] = [];
  for (const id of sourceIds) {
    if (!id || result.some(item => item.source_id === id)) continue;
    const listed = knownSources(read).find(source => source.source_id === id);
    const known = listed ?? (view?.source?.source_id === id ? view.source : view?.related_sources.find(source => source.source_id === id));
    if (known?.revision != null) result.push({ source_id: id, revision: known.revision });
  }
  return result;
}

export type TestRecord = {
  outcome: 'passed' | 'failed'; code?: string; duration?: number | null; at: number;
  tested_configuration_digest?: string; tested_credential_state_revision?: string | null; savedBeforeTest?: boolean;
};
/** `unverified`: a prior pass whose current credential state could not be read back; never shown as passed. */
export type TestPresentation = 'untested' | 'testing' | 'passed' | 'failed' | 'stale' | 'unverified';
export function testRecord(result: DecisionServiceTestResult, at: number): TestRecord {
  return { outcome: result.outcome, code: result.failure_code ?? undefined, duration: result.duration_millis, at,
    tested_configuration_digest: result.tested_configuration_digest, tested_credential_state_revision: result.tested_credential_state_revision };
}
/** A pass is current only for the configuration and credential state it admitted, checked against a successful readback. */
export function testPresentation(record: TestRecord | undefined, view: DecisionServiceView, testing: boolean, current = true): TestPresentation {
  if (testing) return 'testing';
  if (!record) return 'untested';
  if (record.outcome === 'failed') return 'failed';
  const changed = (record.tested_configuration_digest !== undefined && record.tested_configuration_digest !== view.configuration_digest)
    || (record.tested_credential_state_revision !== undefined && record.tested_credential_state_revision !== view.credential_state_revision);
  return changed ? 'stale' : current ? 'passed' : 'unverified';
}
export function testKey(service: Pick<DecisionServiceRecord, 'id' | 'revision'>): string { return `${service.id}/${service.revision}`; }

export type RepairAction = 'manage_source' | 'choose_source' | 'edit' | 'retry' | 'refresh';
/** Maps facts to repair actions without asserting an unproven cause. */
export function repairActions(code: string | undefined, shared: boolean): RepairAction[] {
  switch (code) {
    case 'DECISION_SOURCE_DISABLED':
    case 'DECISION_NO_USABLE_CREDENTIALS': return shared ? ['manage_source'] : ['edit'];
    case 'DECISION_SOURCE_NOT_FOUND':
    case 'DECISION_SOURCE_LINEAGE_MISMATCH': return ['choose_source'];
    case 'DECISION_SOURCE_UNAVAILABLE': return ['retry'];
    case 'DECISION_AUTHENTICATION_REJECTED': return shared ? ['manage_source', 'edit'] : ['edit'];
    case 'DECISION_INVALID_CONFIGURATION':
    case 'DECISION_INPUT_REJECTED':
    case 'DECISION_MALFORMED_OUTPUT': return ['edit'];
    case 'DECISION_TIMEOUT':
    case 'DECISION_RATE_LIMITED':
    case 'DECISION_TRANSPORT_UNAVAILABLE': return ['retry'];
    case 'DECISION_REVISION_CONFLICT': return ['refresh'];
    default: return ['retry', 'edit'];
  }
}

/** Selector labels add model/source/host, then a short source ID, only while names remain ambiguous. */
export function serviceChoiceLabels(views: DecisionServiceView[], language: 'zh' | 'en'): Map<string, string> {
  const labels = new Map<string, string>();
  const levels = [
    (view: DecisionServiceView) => view.service.name,
    (view: DecisionServiceView) => [view.service.name, recordModel(view.service), view.source ? sourceName(view.source, language) : null].filter(Boolean).join(' · '),
    (view: DecisionServiceView) => [view.service.name, recordModel(view.service), view.source ? sourceName(view.source, language) : null, view.source?.endpoint_host ?? endpointHost(view.service.connection.endpoint)].filter(Boolean).join(' · '),
    (view: DecisionServiceView) => [view.service.name, recordModel(view.service), view.source ? sourceName(view.source, language) : null, view.source?.endpoint_host ?? endpointHost(view.service.connection.endpoint), view.source ? shortId(view.source.source_id) : shortId(view.service.id)].filter(Boolean).join(' · '),
  ];
  for (const view of views) {
    let label = levels[0](view);
    for (const level of levels) {
      label = level(view);
      if (views.filter(other => level(other) === label).length === 1) break;
    }
    labels.set(view.service.id, label);
  }
  return labels;
}

export function sourceStateLabel(state: DecisionSourceSummary['state'], language: 'zh' | 'en'): string {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  switch (state) {
    case 'ready': return t('本地可用', 'Ready locally');
    case 'disabled': return t('已停用', 'Disabled');
    case 'missing_credentials': return t('没有可用 Key', 'No usable key');
    case 'missing': return t('已不存在', 'Missing');
    case 'unavailable': return t('状态暂不可读', 'State unavailable');
  }
}

/** Same-name sources add provider and host, then a short ID only while still ambiguous. */
export function sourceChoiceLabels(sources: DecisionSourceSummary[], language: 'zh' | 'en'): Map<string, string> {
  const labels = new Map<string, string>();
  const levels = [
    (source: DecisionSourceSummary) => [sourceName(source, language), source.provider_label, source.endpoint_host].filter(Boolean).join(' · '),
    (source: DecisionSourceSummary) => [sourceName(source, language), source.provider_label, source.endpoint_host, shortId(source.source_id)].filter(Boolean).join(' · '),
  ];
  for (const source of sources) {
    const base = levels[0](source);
    const label = sources.filter(other => levels[0](other) === base).length > 1 ? levels[1](source) : base;
    labels.set(source.source_id, `${label} — ${sourceStateLabel(source.state, language)}`);
  }
  return labels;
}
