import type {
  ComputeConnectionOption,
  ModelDeclaration,
  NativeReasoning,
  TriState,
  UpstreamProtocol,
} from './types';
import { Disclosure } from '../../ui/Disclosure';

export { safeConnectionErrorCode } from './copy';
export { blankModel } from './state';

export function connectionErrorMessage(code: string, zh: boolean): string {
  if (['REVISION_CONFLICT', 'CHANGE_PREVIEW_STALE', 'application.error.revision_conflict', 'application.error.change_preview_stale'].includes(code)) return zh ? '配置在保存前发生变化，尚未提交。输入已保留，请再次保存。' : 'Configuration changed before submission. Your input is preserved; save again.';
  if (code === 'MODEL_TOOL_CALL_NOT_VERIFIED') return zh ? '该模型未返回有效工具调用，当前不能作为 Agent 工具模型使用。请更换支持工具调用的模型，或修复服务后重新验证。' : 'The model did not return a valid tool call and cannot currently be used as an Agent tool model. Choose a tool-capable model or repair the service and verify again.';
  if (code === 'MODEL_INFERENCE_FAILED') return zh
    ? '工具调用验证未通过，当前不能作为 Agent 工具模型使用。请检查模型、服务和权限后重新验证。'
    : 'Tool calling was not verified, so this model cannot currently be used as an Agent tool model. Check its ID, service and permissions, then verify again.';
  if (code === 'MODEL_SAVE_RECHECK_REQUIRED') return zh
    ? '本次保存未生效。请重新检查接入后再保存；如使用 API Key，请重新输入。模型选择和非敏感配置已保留。'
    : 'This save did not take effect. Check the connection again before saving; re-enter the API key if used. Model choices and non-sensitive settings are retained.';
  if (code === 'MODEL_CONNECTION_FIELDS_REQUIRED') return zh ? '请填写 API Key 和有效的连接地址。' : 'Enter an API key and a valid endpoint.';
  if (code === 'MODEL_CONNECTION_KEY_REQUIRED') return zh ? '请填写 API Key。' : 'Enter an API key.';
  if (code === 'MODEL_CONNECTION_AUTHENTICATION_REJECTED') return zh
    ? '凭据未通过检查。请检查 API Key 后重试，输入已保留。'
    : 'The credential check failed. Check the API key and retry; your input is preserved.';
  if (code === 'MODEL_CONNECTION_TRANSPORT_FAILED') return zh
    ? '无法连接模型服务。请检查地址和服务状态后重试，输入已保留。'
    : 'Could not reach the model service. Check the endpoint and service status; your input is preserved.';
  if (code === 'CHECK_CORRELATION_MISMATCH') return zh
    ? '连接状态已变化，请重新检查。输入已保留。'
    : 'The connection changed while it was being checked. Check it again; your input is preserved.';
  if (code === 'MODEL_CAPABILITIES_REQUIRED') return zh
    ? '请完整填写模型能力；不支持的能力也需要明确选择。'
    : 'Complete every model capability, including capabilities that are not supported.';
  if (code === 'MODEL_ID_DUPLICATE') return zh ? '同一接入中的模型 ID 不能重复。' : 'Model IDs must be unique within this connection.';
  if (code === 'MODEL_ID_REQUIRED') return zh ? '请填写服务使用的模型 ID。' : 'Enter the model ID used by the service.';
  if (code === 'MODEL_LIMITS_INVALID') return zh
    ? '上下文和输出上限需为正整数，且输出不能超过上下文。'
    : 'Use positive integer limits, and keep output within the context limit.';
  if (code === 'MODEL_REASONING_INVALID') return zh
    ? '请填写有效的思考档位或预算范围。'
    : 'Enter valid reasoning levels or a valid budget range.';
  if (code === 'MODEL_SELECTION_REQUIRED') return zh ? '至少选择一个模型。' : 'Select at least one model.';
  if (code === 'MODEL_DIRECTORY_EMPTY') return zh
    ? '没有读取到可接入的模型。请检查服务地址，或使用自定义 API 补充未知模型。'
    : 'No connectable model was found. Check the endpoint or use Custom API to describe an unknown model.';
  if (code === 'MODEL_DIRECTORY_UNAVAILABLE') return zh
    ? '模型目录暂时无法读取。请稍后重试，当前 API Key 已保留。'
    : 'The model catalog is temporarily unavailable. Try again; your API key is retained.';
  if (code === 'MODEL_CONNECTION_OPTIONS_UNAVAILABLE') return zh
    ? '暂时无法读取受支持的接入方式。请确认本机服务正在运行后重试。'
    : 'Supported connections could not be loaded. Make sure the local service is running and retry.';
  if (code === 'MODEL_CONNECTION_OPTION_UNAVAILABLE' || code === 'compute.registered_option_unavailable') return zh
    ? '这个接入在当前版本中暂不可用。你仍可通过高级自定义接入连接兼容服务。'
    : 'This connection is unavailable in this version. You can still use an advanced custom connection.';
  if (code === 'compute.registered_catalog_changed') return zh
    ? '受支持的接入目录已经更新。请重新检查，当前 API Key 已保留。'
    : 'The supported connection catalog changed. Check again; your API key is retained.';
  if (code === 'compute.registered_source_mismatch') return zh
    ? '当前凭据不属于这个接入。请返回模型页，从对应接入管理凭据。'
    : 'This credential does not belong to the selected connection. Manage it from the matching connection.';
  return zh
    ? '暂时无法完成操作。请重试，当前输入已保留。'
    : 'The operation could not complete. Try again; your entries are retained.';
}

export function registeredConnectionLabel(option: ComputeConnectionOption | null, zh: boolean) {
  return {
    title: option?.display_name ?? (zh ? '受支持的 API' : 'Supported API'),
    detail: zh ? '由 HiRoute 内置目录提供' : 'From the built-in HiRoute catalog',
  };
}

function booleanFact(value: TriState): { value: boolean | null; basis: 'user_declared' | 'unknown' } {
  if (value === 'unknown') return { value: null, basis: 'unknown' };
  return { value: value === 'supported', basis: 'user_declared' };
}

function triState(value: boolean | null): TriState {
  return value === null ? 'unknown' : value ? 'supported' : 'unsupported';
}

function numberFact(value: string): { value: number | null; basis: 'user_declared' | 'unknown' } {
  const parsed = Number(value);
  return value !== '' && Number.isSafeInteger(parsed) && parsed > 0
    ? { value: parsed, basis: 'user_declared' }
    : { value: null, basis: 'unknown' };
}

function reasoningParameter(protocol: UpstreamProtocol, kind: NativeReasoning['kind']): string {
  if (protocol === 'messages') return kind === 'budget' ? 'thinking.budget_tokens' : 'thinking.type';
  if (protocol === 'responses') return kind === 'budget' ? 'reasoning.max_output_tokens' : 'reasoning.effort';
  return kind === 'budget' ? 'thinking_budget' : 'reasoning_effort';
}

export function reasoningValue(
  kind: NativeReasoning['kind'],
  protocol: UpstreamProtocol,
  previous?: Exclude<NativeReasoning, { kind: 'unknown' }> | null,
): Exclude<NativeReasoning, { kind: 'unknown' }> | null {
  if (kind === 'unknown') return null;
  if (kind === 'fixed') return { kind, profile: previous?.kind === kind ? previous.profile : 'provider-default' };
  if (kind === 'toggle') return { kind, parameter: reasoningParameter(protocol, kind) };
  if (kind === 'discrete') return { kind, parameter: reasoningParameter(protocol, kind), profiles: previous?.kind === kind ? previous.profiles : [] };
  return {
    kind,
    parameter: reasoningParameter(protocol, kind),
    minimum_tokens: previous?.kind === kind ? previous.minimum_tokens : 1,
    maximum_tokens: previous?.kind === kind ? previous.maximum_tokens : 1,
    step_tokens: previous?.kind === kind ? previous.step_tokens : 1,
  };
}

export function manualModelError(model: ModelDeclaration | undefined): string {
  if (!model?.upstream_model_id.trim()) return 'MODEL_ID_REQUIRED';
  const context = model.capabilities.context_tokens.value;
  const output = model.capabilities.max_output_tokens.value;
  if ((context !== null && (!Number.isSafeInteger(context) || context <= 0)) || (output !== null && (!Number.isSafeInteger(output) || output <= 0)) || (context !== null && output !== null && output > context)) return 'MODEL_LIMITS_INVALID';
  const reasoning = model.capabilities.native_reasoning.value;
  if (reasoning?.kind === 'discrete' && (
    !reasoning.profiles.length
    || reasoning.profiles.length > 16
    || new Set(reasoning.profiles).size !== reasoning.profiles.length
    || reasoning.profiles.some(profile => !/^[A-Za-z0-9._-]{1,64}$/.test(profile) || profile.toLocaleLowerCase() === 'ultra')
  )) return 'MODEL_REASONING_INVALID';
  if (reasoning?.kind === 'budget' && (
    ![reasoning.minimum_tokens, reasoning.maximum_tokens, reasoning.step_tokens].every(value => Number.isSafeInteger(value) && value > 0 && value <= 0xffff_ffff)
    || reasoning.maximum_tokens < reasoning.minimum_tokens
    || (output !== null && reasoning.maximum_tokens > output)
    || (reasoning.maximum_tokens - reasoning.minimum_tokens) % reasoning.step_tokens !== 0
  )) return 'MODEL_REASONING_INVALID';
  return '';
}

export function validEndpoint(value: string): boolean {
  try {
    const url = new URL(value);
    return (url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password;
  } catch {
    return false;
  }
}

export function ManualModelEditor({ model, protocol, language, disabled, catalogManaged = false, onChange }: {
  model: ModelDeclaration;
  protocol: UpstreamProtocol;
  language: 'zh' | 'en';
  disabled: boolean;
  catalogManaged?: boolean;
  onChange(next: ModelDeclaration): void;
}) {
  const zh = language === 'zh';
  const updateCapability = <K extends keyof ModelDeclaration['capabilities'],>(key: K, value: ModelDeclaration['capabilities'][K]) =>
    onChange({ ...model, capabilities: { ...model.capabilities, [key]: value } });
  const reasoning = model.capabilities.native_reasoning.value;
  const reasoningKind = reasoning?.kind ?? 'unknown';

  return <fieldset className="connection-fields manual-capability-fields" disabled={disabled}>
    <p className="v3-select-intro">{zh
      ? '填写模型 ID 即可接入基础文字。能力为可选声明，未知能力不会被当作支持。'
      : 'A model ID is enough for basic text. Capabilities are optional declarations; unknown does not mean supported.'}</p>
    <label className="field"><span className="field-label">{zh ? '模型 ID' : 'Model ID'}</span><input className="input" data-autofocus value={model.upstream_model_id} placeholder="my-model" onChange={event => onChange({ ...model, upstream_model_id: event.target.value, display_name: model.display_name === model.upstream_model_id ? event.target.value : model.display_name })} /></label>
    {catalogManaged ? <p className="field-help">{zh
      ? '此模型采用内置目录的名称和能力。如需修改，请返回连接信息，选择“自定义此模板的连接配置”。'
      : 'This model uses its catalog name and capabilities. To edit them, return to connection settings and choose “Customize this template connection”.'}</p> : <>
    <label className="field"><span className="field-label">{zh ? '模型名称（可选）' : 'Model name (optional)'}</span><input className="input" value={model.display_name} placeholder={model.upstream_model_id} onChange={event => onChange({ ...model, display_name: event.target.value })} /></label>
    <Disclosure language={language} label={zh ? '能力与限制（可选）' : 'Capabilities and limits (optional)'}>
    <div className="oc-field-grid">
      {([
        ['tool', zh ? '工具调用' : 'Tool calling'],
        ['vision', zh ? '图片输入' : 'Image input'],
        ['streaming', zh ? '流式输出' : 'Streaming'],
      ] as const).map(([key, label]) => <label className="field" key={key}><span className="field-label">{label}</span><select className="select" value={triState(model.capabilities[key].value)} onChange={event => updateCapability(key, booleanFact(event.target.value as TriState))}><option value="unknown">{zh ? '未知' : 'Unknown'}</option><option value="supported">{zh ? '支持' : 'Supported'}</option><option value="unsupported">{zh ? '不支持' : 'Not supported'}</option></select></label>)}
      <label className="field"><span className="field-label">{zh ? '思考能力' : 'Reasoning capability'}</span><select className="select" value={reasoningKind} onChange={event => {
        const value = reasoningValue(event.target.value as NativeReasoning['kind'], protocol, reasoning);
        updateCapability('native_reasoning', { value, basis: value ? 'user_declared' : 'unknown' });
      }}><option value="unknown">{zh ? '未知' : 'Unknown'}</option><option value="fixed">{zh ? '供应商固定' : 'Provider fixed'}</option><option value="toggle">{zh ? '开关' : 'On / off'}</option><option value="discrete">{zh ? '可选档位' : 'Discrete levels'}</option><option value="budget">{zh ? 'Token 预算' : 'Token budget'}</option></select></label>
      <label className="field"><span className="field-label">{zh ? '上下文上限 · tokens' : 'Context limit · tokens'}</span><input className="input" type="number" min="1" value={model.capabilities.context_tokens.value ?? ''} onChange={event => updateCapability('context_tokens', numberFact(event.target.value))} /></label>
      <label className="field"><span className="field-label">{zh ? '输出上限 · tokens' : 'Output limit · tokens'}</span><input className="input" type="number" min="1" value={model.capabilities.max_output_tokens.value ?? ''} onChange={event => updateCapability('max_output_tokens', numberFact(event.target.value))} /></label>
    </div>
    {reasoning?.kind === 'discrete' && <label className="field"><span className="field-label">{zh ? '支持的档位 · 从低到高，逗号分隔' : 'Supported levels · low to high, comma separated'}</span><input className="input" value={reasoning.profiles.join(', ')} placeholder="low, medium, high" onChange={event => updateCapability('native_reasoning', { value: { ...reasoning, profiles: event.target.value.split(',').map(value => value.trim()).filter(Boolean) }, basis: 'user_declared' })} /></label>}
    {reasoning?.kind === 'budget' && <div className="oc-field-grid">
      {([
        ['minimum_tokens', zh ? '最小思考预算' : 'Minimum reasoning budget'],
        ['maximum_tokens', zh ? '最大思考预算' : 'Maximum reasoning budget'],
        ['step_tokens', zh ? '预算步长' : 'Budget step'],
      ] as const).map(([key, label]) => <label className="field" key={key}><span className="field-label">{label}</span><input className="input" type="number" min="1" value={reasoning[key]} onChange={event => updateCapability('native_reasoning', { value: { ...reasoning, [key]: Number(event.target.value) }, basis: 'user_declared' })} /></label>)}
    </div>}
    </Disclosure>
    </>}
  </fieldset>;
}
