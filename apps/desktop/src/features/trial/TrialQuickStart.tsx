import { readCurrentCatalog } from '../model-feed/catalog-read';
import { randomId } from '../../random-id.ts';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { Dialog, Disclosure, ProductPage, UiIcon } from '../../ui';
import { requestEditorReplacement } from '../../ui/discard-guard';
import { type Editor, type Options, type Plan, type Selection } from '../../plan-editor';
import type { DesktopOperation, DesktopSnapshot } from '../../product/home-projections';
import type { ManagementSnapshot } from '../models/types';
import type { OperationReference } from '../model-connections/types';
import type { AvailableModel, ModelAvailabilityResult } from '../model-feed/types';
import { agentDisplayName } from '../agents/ecosystems';
import { DecisionSelector } from '../decision-services/DecisionSelector';
import { DecisionServicesPage } from '../decision-services/DecisionServicesPage';
import { emptyService, isCustomExtension, type Classifier, type DecisionService } from '../decision-services/types';
import type { DecisionIntent, OpenDecisionConnection } from '../decision-services/presentation';
import { trustLevels } from '../trust/levels';
import { agentEcosystem } from '../agents/ecosystems';
import { TrialConnectModel } from './TrialConnectModel';
import { TrialAgentChoice, TrialAgentMapping } from './TrialAgentMapping';
import { TrialReview } from './TrialReview';
import { createTrialAlias, currentPrimary, initialTrialMapping, trialCriteria, trialSelection, type TrialMapping } from './model';
import { FeedDisclosure } from '../model-feed/FeedDisclosure';
import { factTags, publisherName, rollingIdentity, shortDate } from '../model-feed/presentation';
import './trial.css';
import { protectionExplanation } from '../trust/protection-copy';
import { agentEditorSeed } from '../agents/editor-state';
import { useTrialAgents } from './useTrialAgents';
import { ModelManagementPage } from '../../product/ModelManagementPage';
import { modelAnnouncements } from '../model-feed/announcements';
import { ReasoningDialog } from '../../ui/ReasoningDialog';

const ignoreRepresentedOperations = (_ids: string[]) => {};

export function TrialQuickStart({ initialModelId, initialBindingId, language, mutable, snapshot, operation, onOperation, onRefresh, onClose, onOpenAgent, onOpenPerformance, onOpenSessions, onRepresentedOperations = ignoreRepresentedOperations }: {
  initialModelId?: string; initialBindingId?: string; language: 'zh' | 'en'; mutable: boolean; snapshot: DesktopSnapshot | null; operation: DesktopOperation | null;
  onOpenPerformance?(plan: Plan): void; onOpenSessions?(sessionId?: string): void;
  onRepresentedOperations?(ids: string[]): void;
  onOperation(operation: DesktopOperation | null): void; onRefresh(): Promise<void>; onClose(): void; onOpenAgent(id: string): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [data, setData] = useState<{ availability: ModelAvailabilityResult; options: Options; management: ManagementSnapshot } | null>(null);
  const [trialId, setTrialId] = useState(initialModelId ?? '');
  const [primaryId, setPrimaryId] = useState('');
  const [primarySelection, setPrimarySelection] = useState<Selection>();
  const [trialChosen, setTrialChosen] = useState<Selection>();
  const [reasoningRole, setReasoningRole] = useState<'trial' | 'primary' | null>(null);
  const [trialBinding, setTrialBinding] = useState(''), [primaryBinding, setPrimaryBinding] = useState('');
  const [mapping, setMapping] = useState<TrialMapping>({ agentId: '', presets: ['opus'] });
  const agentRead = useTrialAgents(mapping.agentId);
  const [classifier, setClassifier] = useState<Classifier>(() => ({ kind: 'decision_service', service: emptyService() }));
  const custom = isCustomExtension(classifier);
  const [services, setServices] = useState<DecisionService[]>([]);
  const [connect, setConnect] = useState<AvailableModel | null>(null), [subscriptions, setSubscriptions] = useState(false);
  const [decision, setDecision] = useState<DecisionIntent | null>(null);
  const decisionSelect = useRef<((service: DecisionService) => void) | null>(null);
  const [savedConnections, setSavedConnections] = useState<string[]>([]);
  const [persistencePending, setPersistencePending] = useState(false);
  const [review, setReview] = useState<Editor | null>(null), [published, setPublished] = useState<Plan | null>(null);
  const [catalogLoading, setLoading] = useState(true), [error, setError] = useState('');
  const loading = catalogLoading || !agentRead.ready;
  const [name, setName] = useState<string | null>(null);
  const [choosingPrimary, setChoosingPrimary] = useState(false), [allPrimary, setAllPrimary] = useState(false);
  const [mapped, setMapped] = useState(false);
  const initialized = useRef(false);
  const inFlight = useRef(false);
  const [nonce] = useState(() => randomId().replaceAll('-', '').slice(0, 8));
  async function refresh(rediscover = false) {
    if (inFlight.current) return;
    inFlight.current = true; setLoading(true); setError('');
    try {
      if (rediscover) void agentRead.refresh(true);
      const [availability, options, management, decisions] = await readCurrentCatalog(() => Promise.all([
        invoke<ModelAvailabilityResult>('model_availability'),
        invoke<Options>('plan_editor_options', { input: { authoring: {}, requirements: {}, native_selections: {}, suggest_free: false } }),
        invoke<ManagementSnapshot>('compute_management_snapshot'),
        invoke<{ services: DecisionService[] }>('decision_services'),
      ]));
      setData({ availability, options, management }); setServices(decisions.services);
      if (!initialized.current) {
        initialized.current = true;
        const requested = options.candidates.find(c => c.binding_id === initialBindingId && c.routable);
        if (requested) { setTrialId(requested.model_configuration_id); setTrialBinding(requested.binding_id); }
        if (decisions.services.length === 1) setClassifier({ kind: 'decision_service', service: decisions.services[0] });
      }
      return { availability, options, management };
    } catch { setError(t('试用所需信息暂时无法读取。请重试，已完成的配置仍保留。', 'Trial setup information could not be read. Retry; completed configuration is retained.')); }
    finally { inFlight.current = false; setLoading(false); }
  }
  useEffect(() => { void refresh(); }, []);
  const mappingInitialized = useRef(false);
  const primaryOrigin = useRef<{ kind: 'unset' | 'agent' | 'manual'; agentId: string }>({ kind: 'unset', agentId: '' });
  useEffect(() => {
    const agents = agentRead.snapshot;
    if (!data || !agentRead.ready || !agents) return;
    // Choosing Primary while details load must not suppress the default Agent;
    // choosing an Agent explicitly must not be undone by a late default read.
    if (!mappingInitialized.current) {
      mappingInitialized.current = true;
      setMapping(value => value.agentId ? value : initialTrialMapping(agents));
    }
    const agentId = mapping.agentId || agents.selected?.agent_id || '';
    if (primaryOrigin.current.kind === 'manual' && !primaryOrigin.current.agentId) primaryOrigin.current.agentId = agentId;
    if (primaryOrigin.current.kind !== 'unset' || !agentId || agents.selected?.agent_id !== agentId) return;
    const current = currentPrimary(agents.selected ?? undefined, agents.plans.plans);
    const candidate = data.options.candidates.find(c => c.binding_id === current?.binding_id && c.routable);
    if (candidate && candidate.model_configuration_id !== trialId) {
      primaryOrigin.current = { kind: 'agent', agentId };
      setPrimaryId(candidate.model_configuration_id); setPrimaryBinding(candidate.binding_id); setPrimarySelection(current);
    }
  }, [data, agentRead.ready, agentRead.snapshot, mapping.agentId, trialId]);
  const openDecision: OpenDecisionConnection = (kind, select) => { decisionSelect.current = select; setDecision({ key: randomId(), kind }); };
  const operationRef = (value: OperationReference) => onOperation({ ...value, safe_error_code: null });
  type TrialModel = Pick<AvailableModel, 'model_configuration_id' | 'display_name' | 'identity_stability' | 'availability'> & { catalog?: AvailableModel };
  const models: TrialModel[] = (data?.availability.models ?? []).filter(m => m.origin.kind !== 'published').map(m => ({ ...m, catalog: m }));
  for (const candidate of data?.options.candidates ?? []) {
    if (candidate.routable && !models.some(m => m.model_configuration_id === candidate.model_configuration_id)) models.push({ model_configuration_id: candidate.model_configuration_id, display_name: candidate.display_name, identity_stability: 'unknown', availability: { kind: 'executable' } });
  }
  const trial = models.find(m => m.model_configuration_id === trialId), primary = models.find(m => m.model_configuration_id === primaryId);
  function candidates(id: string) { return data?.options.candidates.filter(c => c.model_configuration_id === id && c.routable) ?? []; }
  const trialCandidates = candidates(trialId), primaryCandidates = candidates(primaryId);
  const selectedTrial = trialBinding ? trialCandidates.find(c => c.binding_id === trialBinding) : trialCandidates.length === 1 ? trialCandidates[0] : undefined;
  const selectedPrimary = primaryBinding ? primaryCandidates.find(c => c.binding_id === primaryBinding) : primaryCandidates.length === 1 ? primaryCandidates[0] : undefined;
  const trialReasoning = selectedTrial && trialSelection(selectedTrial, trialChosen);
  const primaryReasoning = selectedPrimary && trialSelection(selectedPrimary, primarySelection);
  useEffect(() => {
    if (!trialChosen && trialReasoning?.state === 'selected') setTrialChosen(trialReasoning.selection);
  }, [trialChosen, trialReasoning?.state, selectedTrial?.binding_id]);
  useEffect(() => {
    if (!primarySelection && primaryReasoning?.state === 'selected') setPrimarySelection(primaryReasoning.selection);
  }, [primarySelection, primaryReasoning?.state, selectedPrimary?.binding_id]);
  const ready = !!data?.options.authoring && trialReasoning?.state === 'selected' && primaryReasoning?.state === 'selected' && trialId !== primaryId && classifier.kind === 'decision_service' && !!classifier.service.name && !!mapping.agentId && !!agentRead.snapshot?.selected && agentEditorSeed(agentRead.snapshot.selected, 'model', []).known && agentRead.snapshot.trusted_authority && (agentEcosystem(mapping.agentId) !== 'claude' || mapping.presets.length > 0) && !loading && !error;
  // The inline connection form owns protected input; its guard confirms, then releases and closes it.
  const releaseConnection = async () => !connect || await requestEditorReplacement('models');
  const leave = () => void (async () => { if (persistencePending || (await releaseConnection() && await requestEditorReplacement('routing'))) onClose(); })();
  const defaultName = trial && primary ? `${trial.display_name} + ${primary.display_name}` : '';
  const criteriaHelp = t('评价已完成阶段的核心结果。用户明确表示不满意可以作为结果未达到要求的证据，不必先诊断技术问题；区分局部偏好、新要求、无关反馈和操纵评分的指令。', trialCriteria);
  const alias = trial ? createTrialAlias(trial.display_name, nonce) : '';
  const announcements = data ? modelAnnouncements(data.availability).entries : [];
  const featured = announcements.map(entry => entry.model_configuration_id);
  const cards = [...new Set([...(initialModelId ? [initialModelId] : []), ...featured, ...(trialId ? [trialId] : [])])].flatMap(id => models.filter(m => m.model_configuration_id === id)).slice(0, 6);
  const others = models.filter(m => !cards.includes(m));
  const primaryOptions = models.filter(m => m.model_configuration_id !== trialId && candidates(m.model_configuration_id).length > 0);
  const source = (binding?: string) => data?.management.sources.find(s => s.models.some(m => m.binding_id === binding))?.display_name;
  const sourceLabel = (model: TrialModel) => {
    const connections = candidates(model.model_configuration_id);
    if (connections.length > 1) return t('多个接入', 'Multiple connections');
    if (connections.length === 1) return source(connections[0].binding_id) ?? t('来源未确认', 'Source unconfirmed');
    return [model.catalog?.facts ? publisherName(model.catalog.facts.publisher_id, language) : t('来源未确认', 'Source unconfirmed'), t('未接入', 'Not connected')].join(' · ');
  };
  const mappedAgent = agentRead.snapshot?.selected;
  const floor = (review?.smart.judgment.competence.floor_millis ?? data?.options.authoring?.editor.smart.judgment.competence.floor_millis ?? 500) / 1000;
  const missing = loading ? '' : !trialId ? t('先选择要试用的模型', 'Choose a model to try')
    : !selectedTrial ? trial?.availability.kind === 'needs_newer_client' ? t('这个模型需要更新 HiRoute', 'This model needs a newer HiRoute') : trialCandidates.length > 1 ? t('选择试用模型使用的接入', 'Choose the connection for the trial model') : t(`先接入 ${trial?.display_name ?? ''}`, `Connect ${trial?.display_name ?? ''} first`)
    : !selectedPrimary ? t('选择主力模型及可用接入', 'Choose a Primary model and available connection')
    : trialReasoning?.state !== 'selected' || primaryReasoning?.state !== 'selected' ? t('请选择有效的思考设置', 'Choose valid reasoning settings')
    : classifier.kind !== 'decision_service' || !classifier.service.name ? t('选择决策模型', 'Choose a decision model')
    : !mapping.agentId ? t('选择要接入的 Agent', 'Choose an Agent')
    : !mapping.presets.length && agentEcosystem(mapping.agentId) === 'claude' ? t('至少选择一个预设', 'Choose at least one preset') : '';
  function cardState(model: TrialModel) {
    if (model.availability.kind === 'needs_newer_client') return { tone: 'warn', text: t(`需要 HiRoute ${model.availability.version}`, `Requires HiRoute ${model.availability.version}`) };
    if (candidates(model.model_configuration_id).length) return { tone: 'good', text: t('已连接', 'Connected') };
    return { tone: '', text: t('需要先连接，下面可以直接完成', 'Connect it first, right below') };
  }
  function connectionPicker(role: 'trial' | 'primary') {
    const available = role === 'trial' ? trialCandidates : primaryCandidates;
    const selected = role === 'trial' ? selectedTrial : selectedPrimary;
    if (available.length < 2) return null;
    const label = (candidate: typeof available[number]) => `${source(candidate.binding_id) ?? t('来源未确认', 'Source unconfirmed')} · ${candidate.billing_class}`;
    return <label className="field trial-connection-field"><span className="field-label">{t('使用的接入', 'Connection')}</span><select className="input" value={selected?.binding_id ?? ''} onChange={e => { if (role === 'trial') { setTrialBinding(e.target.value); setTrialChosen(undefined); } else { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimaryBinding(e.target.value); setPrimarySelection(undefined); } }}><option value="">{t('请选择', 'Select')}</option>{available.map((c, index) => <option key={c.binding_id} value={c.binding_id}>{label(c)}{available.filter(other => label(other) === label(c)).length > 1 ? ` · ${index + 1}` : ''}</option>)}</select></label>;
  }
  function reasoningPicker(role: 'trial' | 'primary') {
    const candidate = role === 'trial' ? selectedTrial : selectedPrimary;
    const result = role === 'trial' ? trialReasoning : primaryReasoning;
    if (!candidate || !result) return null;
    const value = result.selection.reasoning;
    const label = value?.kind === 'profile' ? value.profile : value?.kind === 'toggle' ? (value.enabled ? t('开启', 'On') : t('关闭', 'Off')) : value?.kind === 'budget' ? `${value.tokens} tokens` : candidate.reasoning.kind === 'fixed' ? t('固定', 'Fixed') : t('请选择', 'Choose');
    return <div className="field trial-reasoning"><span className="field-label">{t('思考设置', 'Reasoning')}</span>{candidate.reasoning.kind === 'fixed' ? <span>{label}</span> : <button type="button" className="btn btn-quiet" onClick={() => setReasoningRole(role)}>{label}</button>}{result.state === 'invalid' && <p className="oc-inline-error" role="alert">{t('当前思考设置或能力信息无效，请检查接入。', 'The reasoning setting or capability is invalid. Check the connection.')}</p>}</div>;
  }
  const chooseTrial = (id: string) => void (async () => {
    if (id === trialId || !(await releaseConnection())) return;
    setTrialId(id); setTrialBinding(''); setTrialChosen(undefined); setConnect(null); if (id === primaryId) { primaryOrigin.current = { kind: 'unset', agentId: mapping.agentId }; setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); }
  })();
  const step = mapped ? 4 : published ? 3 : review ? 2 : 1;
  const steps = [t('选择模型', 'Choose models'), t('核对并发布', 'Review and publish'), t('映射 Agent', 'Map the Agent'), t('开始使用', 'Start using it')];
  const setup = data && <fieldset disabled={catalogLoading || !mutable} className="trial-setup">
    <section className="trial-card">
      <div className="trial-card-head"><h3>{t('要试用的模型', 'Model to try')}</h3>{featured.length > 0 && <span>{t('来自 hiroute.ai 模型目录和内置目录', 'From the hiroute.ai model directory and the built-in catalog')}</span>}</div>
      <div className="trial-model-grid" role="group" aria-label={t('要试用的模型', 'Model to try')}>{cards.map(model => { const state = cardState(model); const facts = model.catalog?.facts ?? null; return <button type="button" key={model.model_configuration_id} className={`trial-model-card${trialId === model.model_configuration_id ? ' selected' : ''}`} aria-pressed={trialId === model.model_configuration_id} disabled={model.availability.kind === 'needs_newer_client'} onClick={() => chooseTrial(model.model_configuration_id)}>
        <span className="trial-model-title"><strong>{model.display_name}</strong>{trialId === model.model_configuration_id && <UiIcon name="check" className="trial-model-check" />}</span>
        <span className="trial-model-meta">{sourceLabel(model)}</span>
        {facts && <span className="model-fact-tags">{factTags(facts, language).map(tag => <span key={tag} className="badge no-dot">{tag}</span>)}</span>}
        <span className="trial-model-note new-model-note">{t('从范围明确的小任务开始试用，并观察实际表现。', 'Start with a well-scoped task and observe the results.')}</span>
        <span className={`trial-model-state ${state.tone}`}>{state.text}</span>
      </button>; })}</div>
      {others.length > 0 && <label className="trial-other-model"><span>{t('试用其他模型', 'Try another model')}</span><select className="input" value={cards.some(m => m.model_configuration_id === trialId) ? '' : trialId} onChange={e => e.target.value && chooseTrial(e.target.value)}><option value="">{t('从全部模型中选择', 'Choose from all models')}</option>{others.map(m => <option key={m.model_configuration_id} value={m.model_configuration_id} disabled={m.availability.kind === 'needs_newer_client'}>{m.display_name} · {sourceLabel(m)}{m.availability.kind === 'needs_newer_client' ? ` · HiRoute ${m.availability.version}+` : ''}</option>)}</select></label>}
      {trial && <Disclosure className="trial-model-facts" label={t('版本与来源', 'Version and source')} language={language}>
        {trial.catalog?.origin.kind === 'feed' && <p className="field-help">{t(`模型信息来源：hiroute.ai 模型目录（${shortDate(trial.catalog.origin.as_of, language)} 资料日期，内容已校验）。`, `Model information: the hiroute.ai model directory (information as of ${shortDate(trial.catalog.origin.as_of, language)}, content validated).`)}</p>}
        {rollingIdentity(trial.identity_stability) && <p className="field-help">{t('供应商可能更新同名模型；后续证据可能混合多个版本。', 'The provider may update this model alias; later evidence may mix versions.')}</p>}
        <p className="field-help">{t('目录或公告日期不能证明当前接入实际使用的版本。', 'A catalog or announcement date does not prove the version served by this connection.')}</p>
      </Disclosure>}
      {connectionPicker('trial')}
      {trialBinding && !selectedTrial && <p className="oc-inline-error" role="alert">{t('所选试用接入当前不可用，请重新选择。', 'The selected trial connection is unavailable. Choose again.')}</p>}
      {selectedTrial && <p className="field-help">{t('所选来源', 'Selected source')}: {source(selectedTrial.binding_id) ?? t('来源未确认', 'Source unconfirmed')}</p>}
      {reasoningPicker('trial')}
      {alias && <p className="field-help">{t('路由别名', 'Route alias')}: <code>{alias}</code></p>}
      {trial?.catalog && !trialCandidates.length && trial.availability.kind !== 'needs_newer_client' && data && (connect?.model_configuration_id === trial.model_configuration_id
        ? <div className="trial-connect-card"><TrialConnectModel key={connect.model_configuration_id} model={connect} language={language} management={data.management} mutable={mutable} onOperation={operationRef} onBack={() => { setConnect(null); void refresh(); }} onSaved={(_result, model) => { setSavedConnections(values => [...new Set([...values, model.display_name])]); setConnect(null); void refresh(); void onRefresh(); }} /></div>
        : <div className="trial-connect-card"><div><strong>{t(`先接入 ${trial.display_name}`, `Connect ${trial.display_name} first`)}</strong><p className="field-help">{t('只读取这个服务的模型目录，不会发起付费推理。API Key 保存在本机。', 'Only the service model list is read; no paid inference runs. The API key stays on this device.')}</p></div><button type="button" className="btn btn-primary" onClick={() => setConnect(trial.catalog!)}>{t('在此接入', 'Connect here')}</button></div>)}
      <div className="callout trial-rule"><UiIcon name="info" /><span>{t('新模型先放进常规组。', 'The new model starts in Regular. ')}{protectionExplanation(classifier, language)}{!custom && t('主力拿到低分不会再回退。熟悉之后，可以用分工把难的任务交给新模型。', ' Primary does not fall back after a low score. Once you know the new model, divide work to give it harder tasks.')}</span></div>
    </section>
    <section className="trial-card trial-card-pair">
      <div className="trial-column">
        <h3>{t('主力模型', 'Primary model')}</h3>
        {selectedPrimary && primary && !choosingPrimary ? <button type="button" className="trial-choice selected" aria-expanded={false} onClick={() => setChoosingPrimary(true)}><strong>{primary.display_name}</strong><span>{[source(selectedPrimary.binding_id) ?? t('来源未确认', 'Source unconfirmed'), primaryOrigin.current.kind === 'agent' && mappedAgent ? t(`来自 ${agentDisplayName(mappedAgent.agent_id, language)} 现在使用的模型`, `What ${agentDisplayName(mappedAgent.agent_id, language)} uses now`) : ''].filter(Boolean).join(' · ')}</span><em>{t('更换', 'Change')}</em></button>
          : <div className="trial-choice-list" role="group" aria-label={t('主力模型', 'Primary model')}>{(allPrimary ? primaryOptions : primaryOptions.slice(0, 4)).map(model => <button type="button" key={model.model_configuration_id} className={`trial-choice${primaryId === model.model_configuration_id ? ' selected' : ''}`} aria-pressed={primaryId === model.model_configuration_id} onClick={() => { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimaryId(model.model_configuration_id); setPrimaryBinding(''); setPrimarySelection(undefined); setChoosingPrimary(false); }}><strong>{model.display_name}</strong><span>{sourceLabel(model)}</span></button>)}{!allPrimary && primaryOptions.length > 4 && <button type="button" className="btn btn-quiet trial-inline-action" onClick={() => setAllPrimary(true)}>{t(`显示全部 ${primaryOptions.length} 个已连接模型`, `Show all ${primaryOptions.length} connected models`)}</button>}{!primaryOptions.length && <p className="field-help">{t('还没有其他已连接的模型。', 'No other connected model yet.')}</p>}</div>}
        {connectionPicker('primary')}
        {primaryId && !selectedPrimary && <p className="oc-inline-error" role="alert">{t('所选主力接入当前不可用，请重新选择。', 'The selected Primary connection is unavailable. Choose again.')}</p>}
        {reasoningPicker('primary')}
        <p className="field-help">{custom ? t('扩展选择主力组时使用此模型。仅在已保存配置明确指向单个模型时，才自动填入当前主力。', 'This model is used when the extension selects Primary. It is prefilled only when saved settings identify a single model.') : t('负责复杂任务和需要保护的轮次。仅在已保存配置明确指向单个模型时，才自动填入当前主力。', 'It takes complex tasks and turns that need protection. It is prefilled only when saved settings identify a single model.')}</p>
        <button type="button" className="btn btn-quiet trial-inline-action" onClick={() => setSubscriptions(true)}>{t('复用本机订阅', 'Reuse a local subscription')}</button>
      </div>
      <div className="trial-column trial-decision">
        <DecisionSelector classifier={classifier} smart allowHeuristic={false} services={services} language={language} onChange={setClassifier} onOpenServices={openDecision} />
        <p className="field-help">{custom ? t('扩展直接选择执行组，可以提供仅用于观察的评分。', 'The extension selects the execution group directly and may provide scores for observation.') : t('试用需要给每个阶段打胜任分，所以不提供启发式规则。', 'A trial needs a competence score for every stage, so heuristic rules are not offered.')}</p>
      </div>
    </section>
    <section className="trial-card trial-card-pair">
      <div className="trial-column"><h3>{t('接入的 Agent', 'Agent')}</h3>{agentRead.snapshot && <TrialAgentChoice snapshot={agentRead.snapshot} value={mapping} onChange={value => { mappingInitialized.current = true; if (value.agentId !== mapping.agentId) { primaryOrigin.current = { kind: 'unset', agentId: value.agentId }; setPrimaryId(''); setPrimaryBinding(''); setPrimarySelection(undefined); } setMapping(value); }} language={language} loading={agentRead.loading} onRetry={() => void agentRead.refresh()} />}{agentRead.error && !mapping.agentId && <p role="alert">{t('Agent 配置暂时无法读取，请重试。', 'Agent settings could not be read. Retry.')}<button className="btn btn-quiet" onClick={() => void agentRead.refresh(true)}>{t('重试', 'Retry')}</button></p>}</div>
      <div className="trial-column">
        <h3>{custom ? t('路由名称', 'Route name') : t('名称与起始信任度', 'Name and starting trust')}</h3>
        <label className="field"><span className="field-label">{t('路由名称', 'Route name')}</span><input className="input" value={name ?? defaultName} placeholder={t('选好模型后自动填写', 'Filled in after choosing models')} maxLength={80} onChange={e => setName(e.target.value)} /></label>
        {!custom && <><p className="trial-level"><span className="badge no-dot cyan">{trustLevels[0].label[language]}</span>{t(`简单概率 ≥ ${(trustLevels[0].threshold_millis / 1000).toFixed(2)} · 胜任下限 ${floor}`, `P(simple) ≥ ${(trustLevels[0].threshold_millis / 1000).toFixed(2)} · Competence floor ${floor}`)}</p>
        <p className="field-help">{t('评分完整的阶段、评分覆盖和会话分布会影响升档建议；保持这一档也是正常结果。', 'Fully scored stages, scoring coverage and session spread inform suggestions to step up. Staying at this level is a normal outcome.')}</p>
        <Disclosure className="trial-criteria" label={t('胜任评分标准', 'Competence criteria')} language={language}><p>{criteriaHelp}</p><p className="field-help">{t('下一步核对时可以修改实际提示词。', 'You can edit the actual prompt in the next step.')}</p></Disclosure></>}
      </div>
    </section>
    <div className="trial-footer">
      <span className="trial-footer-hint" role="status">{missing}</span>
      <button type="button" className="btn btn-quiet" onClick={() => void refresh(true)}>{t('刷新可用状态', 'Refresh availability')}</button>
      <button type="button" className="btn" onClick={leave}>{t('取消', 'Cancel')}</button>
      <button type="button" className="btn btn-primary" disabled={!ready} onClick={() => {
        if (!ready || !selectedTrial || !selectedPrimary || !trialReasoning || !primaryReasoning || classifier.kind !== 'decision_service') return;
        if (!data?.options.authoring) return;
        const editor = structuredClone(data.options.authoring.editor); editor.mode = 'smart_saving'; editor.custom_alias = alias;
        editor.display_name = (name ?? defaultName).trim() || defaultName;
        editor.purpose = custom ? t('由自定义扩展选择执行组，观察新模型在自己任务中的表现。', 'Let the custom extension choose the execution group and observe the new model on your own tasks.') : t('在主力保护下试用新模型，根据自己的任务证据调整路由。', 'Try a new model with Primary protection and adjust routing using evidence from your own tasks.');
        editor.smart.economy = [trialReasoning.selection]; editor.smart.primary = [primaryReasoning.selection];
        editor.smart.classifier = structuredClone(classifier);
        if (!custom) { editor.smart.judgment.competence.standard = 'custom'; editor.smart.judgment.competence.criteria[0] = trialCriteria; editor.smart.judgment.degree.simple_threshold_millis = trustLevels[0].threshold_millis; }
        setReview(editor);
      }}>{t('检查改动', 'Review changes')}</button>
    </div>
  </fieldset>;
  return <ProductPage className="trial-page" title={t('试用新模型', 'Try a new model')} subtitle={custom ? t('由自定义扩展选择执行组。发布前会列出全部改动。', 'Your custom extension chooses the execution group. Every change is listed before publishing.') : t('新模型先上，主力兜底。发布前会列出全部改动。', 'The new model goes first and Primary catches what it misses. Every change is listed before publishing.')} actions={<button type="button" className="btn btn-quiet" onClick={leave}><UiIcon name="arrowLeft" />{t('返回', 'Back')}</button>}>
    <div className="trial-wizard">
      <ol className="trial-steps" aria-label={t('试用步骤', 'Trial steps')}>{steps.map((label, i) => <li key={label} className={i + 1 < step ? 'done' : i + 1 === step ? 'current' : undefined} aria-current={i + 1 === step ? 'step' : undefined}><span>{i + 1}</span>{label}</li>)}</ol>
      {data?.availability.models.some(model => model.origin.kind === 'feed') && <FeedDisclosure language={language} mutable={mutable} />}
      {error && <div className="callout warn" role="alert"><p>{error}</p><button className="btn" onClick={() => void refresh()}>{t('重试', 'Retry')}</button></div>}
      {savedConnections.length > 0 && <p className="trial-saved" role="status">{t('本次已保存接入：', 'Connections saved in this flow: ')}{savedConnections.join(' · ')}</p>}
      {published ? <section className="trial-card"><TrialAgentMapping onConfirmed={() => setMapped(true)} onOpenPerformance={() => onOpenPerformance?.(published)} onOpenSessions={onOpenSessions} plan={published} initial={mapping} initialChoices={agentRead.snapshot} language={language} mutable={mutable} onOperation={operationRef} onChanged={() => void onRefresh()} onOpenAgent={onOpenAgent} /></section>
        : review ? <section className="trial-card trial-review-card"><div className="trial-card-head"><h3>{t('核对改动', 'Review changes')}</h3><span>{t('将映射', 'Mapping to review')}: {agentDisplayName(mapping.agentId, language)}{agentEcosystem(mapping.agentId) === 'claude' ? ` · ${mapping.presets.join(', ')}` : ''}</span></div><TrialReview editor={review} previewAgentId={mapping.agentId} language={language} mutable={mutable} snapshot={snapshot} operation={operation} onOperation={onOperation} onRefresh={onRefresh} onPublished={setPublished} onPendingChange={setPersistencePending} onRepresentedOperations={onRepresentedOperations} onClose={onClose} onOpenServices={openDecision} /></section>
        : setup || (!error && <p role="status">{t('正在读取配置…', 'Reading configuration…')}</p>)}
    </div>
    {reasoningRole && (() => { const candidate = reasoningRole === 'trial' ? selectedTrial : selectedPrimary; const result = reasoningRole === 'trial' ? trialReasoning : primaryReasoning; return candidate && result ? <ReasoningDialog name={candidate.display_name} native={candidate.reasoning} value={result.selection.reasoning} language={language} onClose={() => setReasoningRole(null)} onApply={reasoning => { const selection = { binding_id: candidate.binding_id, reasoning }; if (reasoningRole === 'trial') setTrialChosen(selection); else { primaryOrigin.current = { kind: 'manual', agentId: mapping.agentId }; setPrimarySelection(selection); } setReasoningRole(null); }} /> : null; })()}
    {decision && <Dialog open title={t('接入决策连接', 'Connect a decision service')} closeLabel={t('返回试用', 'Back to trial')} onClose={() => void (async () => { if (await requestEditorReplacement('decisions')) setDecision(null); })()} className="trial-dialog"><DecisionServicesPage language={language} active mutable={mutable} intent={decision} plans={snapshot?.catalog.plans} drafts={snapshot?.catalog.drafts} onChanged={() => void refresh()} onReturn={() => setDecision(null)} onUse={service => { decisionSelect.current?.(service); setDecision(null); void refresh(); }} /></Dialog>}
    {subscriptions && <Dialog open title={t('复用本机订阅', 'Reuse a local subscription')} closeLabel={t('返回试用', 'Back to trial')} onClose={() => void (async () => { if (await requestEditorReplacement('models')) { setSubscriptions(false); void refresh(); } })()} className="trial-dialog"><ModelManagementPage agents={agentRead.snapshot?.choices} language={language} active trustedAuthority={mutable} startScanning onOperation={operationRef} onRecoveryRefresh={() => void refresh()} onChanged={() => { void refresh(); void onRefresh(); }} plans={snapshot?.catalog.plans} /></Dialog>}
  </ProductPage>;
}
