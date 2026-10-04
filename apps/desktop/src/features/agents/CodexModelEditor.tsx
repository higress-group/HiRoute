import { agentModelStatus } from './status';
import React from 'react';
import { Disclosure, UiIcon } from '../../ui';
import type { AgentModelEditorProps } from './AgentModelEditor';
import { CodexAccessSettings } from './CodexAccessPanel';
import { codexSurfaceFacts } from './codex-surfaces';
import { surfaceName } from './ecosystems';
import { codexDefaultChoiceValid } from './editor-state';
import type {
  AgentDefaultChoice, AgentFixedModel, AgentNativeModel, AgentNativeReasoning,
  AgentModelSourceCoverage, AgentReasoningSelection, CodexNativeModelMode,
} from './types';

export function CodexModelEditor({
  agent: selected, values: editorValues, plans, language, disabled, codexMode: activeCodexMode,
  onCodexMode, onChange: setEditorValues, onEdited, onCreatePlan, invalid: modelFormInvalid,
}: AgentModelEditorProps) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const enabledPlans = plans.filter(plan => plan.head.status === 'enabled');
  const planName = (id: string) => plans.find(plan => plan.agent_plan_id === id)?.desired.display_name;
  const planEnabled = (id: string) => enabledPlans.some(plan => plan.agent_plan_id === id);
  const protectedNativeModelIds = new Set(agentModelStatus(selected)?.protected_native_model_ids ?? []);
  const codexSurfaces = codexSurfaceFacts(activeCodexMode, selected.available_surfaces ?? []);
  const catalogModels = (selected.native_model_catalog?.models ?? [])
    .map(model => ({ ...model, source_options: model.source_options ?? [] }));
  const fixedModelRows = editorValues.fixedModels.map(fixed => catalogModels
    .find(model => model.client_model_id === fixed.client_model_id) ?? {
      client_model_id: fixed.client_model_id,
      display_name: fixed.client_model_id,
      source_options: [],
    });
  const creatingModelConnection = !agentModelStatus(selected)?.current_selection;
  const defaultChoice = editorValues.defaultChoice;
  const codexDefaultValid = codexDefaultChoiceValid(defaultChoice, editorValues.nativeModelMode,
    selected.native_model_catalog?.native_default_model, editorValues.fixedModels, editorValues.allowedPlanIds);
  const showCodexDefault = editorValues.nativeModelMode === 'preserve_available'
    || editorValues.fixedModels.length > 0 || editorValues.allowedPlanIds.length > 1;
  function toggleAllowedPlan(planId: string) {
    setEditorValues(value => {
      const selected = value.allowedPlanIds.includes(planId);
      const allowedPlanIds = selected
        ? value.allowedPlanIds.filter(id => id !== planId)
        : [...value.allowedPlanIds, planId];
      const defaultChoice = selected
        && value.defaultChoice.kind === 'plan'
        && value.defaultChoice.plan_id === planId
        ? value.nativeModelMode === 'preserve_available'
          ? { kind: 'preserve_native' } as const
          : allowedPlanIds.length ? { kind: 'plan', plan_id: allowedPlanIds[0] } as const
            : value.fixedModels.length ? { kind: 'fixed_model', client_model_id: value.fixedModels[0].client_model_id } as const
              : { kind: 'preserve_native' } as const
        : !selected && value.nativeModelMode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
          ? { kind: 'plan', plan_id: planId } as const
          : value.defaultChoice;
      return { ...value, allowedPlanIds, defaultChoice };
    });
    onEdited();
  }

  function defaultFixedReasoning(reasoning: AgentNativeReasoning): AgentReasoningSelection | undefined {
    if (reasoning.kind === 'fixed') return undefined;
    if (reasoning.kind === 'toggle') return { kind: 'toggle', enabled: true };
    if (reasoning.kind === 'budget') return { kind: 'budget', tokens: reasoning.maximum_tokens };
    const enabled = reasoning.profiles.filter(profile => !['none', 'off', 'disabled'].includes(profile.toLocaleLowerCase()));
    const profile = enabled.at(-1) ?? reasoning.profiles.at(-1);
    return profile ? { kind: 'profile', profile } : undefined;
  }

  function selectFixedSource(model: AgentNativeModel, bindingId: string) {
    setEditorValues(value => {
      const fixedModels = value.fixedModels.filter(item => item.client_model_id !== model.client_model_id);
      const source = model.source_options?.find(option => option.binding_id === bindingId && option.state === 'ready');
      if (source) {
        fixedModels.push({
          client_model_id: model.client_model_id,
          candidate: {
            binding_id: source.binding_id,
            reasoning: defaultFixedReasoning(source.reasoning),
          },
        });
      }
      const defaultChoice = !source
        && value.defaultChoice.kind === 'fixed_model'
        && value.defaultChoice.client_model_id === model.client_model_id
        ? value.nativeModelMode === 'hiroute_only' && value.allowedPlanIds.length
          ? { kind: 'plan', plan_id: value.allowedPlanIds[0] } as const
          : { kind: 'preserve_native' } as const
        : source && value.nativeModelMode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
          ? { kind: 'fixed_model', client_model_id: model.client_model_id } as const
        : value.defaultChoice;
      return { ...value, fixedModels, defaultChoice };
    });
    onEdited();
  }

  function setFixedReasoning(clientModelId: string, reasoning: AgentReasoningSelection | undefined) {
    setEditorValues(value => ({
      ...value,
      fixedModels: value.fixedModels.map(model => model.client_model_id === clientModelId
        ? { ...model, candidate: { ...model.candidate, reasoning } }
        : model),
    }));
    onEdited();
  }

  function defaultChoiceValue(choice: AgentDefaultChoice): string {
    if (choice.kind === 'preserve_native') return 'native';
    return `${choice.kind === 'plan' ? 'plan' : 'fixed'}:${choice.kind === 'plan' ? choice.plan_id : choice.client_model_id}`;
  }

  function selectDefaultChoice(value: string) {
    const defaultChoice: AgentDefaultChoice = value === 'native'
      ? { kind: 'preserve_native' }
      : value.startsWith('plan:')
        ? { kind: 'plan', plan_id: value.slice(5) }
        : { kind: 'fixed_model', client_model_id: value.slice(6) };
    setEditorValues(current => ({ ...current, defaultChoice }));
    onEdited();
  }

  function selectNativeModelMode(mode: CodexNativeModelMode) {
    setEditorValues(value => {
      if (mode === value.nativeModelMode) return value;
      const fixedModels = mode === 'hiroute_only'
        ? value.fixedModels.filter(model => !protectedNativeModelIds.has(model.client_model_id))
        : value.fixedModels;
      const defaultChoice = mode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
        ? value.allowedPlanIds.length ? { kind: 'plan', plan_id: value.allowedPlanIds[0] } as const
          : fixedModels.length ? { kind: 'fixed_model', client_model_id: fixedModels[0].client_model_id } as const
            : value.defaultChoice
        : value.defaultChoice;
      return { ...value, nativeModelMode: mode, fixedModels, defaultChoice };
    });
    onEdited(false);
  }

  const sourceStateLabel = (state: AgentModelSourceCoverage['state']) => state === 'ready'
    ? text('来源与账号已覆盖', 'Source and account covered')
    : state === 'credential_required'
      ? text('需要凭据', 'Credential required')
      : state === 'authorization_required'
        ? text('需要账号授权', 'Account authorization required')
        : state === 'disabled'
          ? text('来源已停用', 'Source disabled')
          : text('模型目录匹配未证明', 'Model directory match unproven');
  const fixedReasoningControl = (
    model: AgentNativeModel,
    fixed: AgentFixedModel,
    source: AgentModelSourceCoverage,
  ) => {
    const reasoning = source.reasoning;
    const selection = fixed.candidate.reasoning;
    if (reasoning.kind === 'fixed') {
      return <span className="field-help">{text('原生思考设置：', 'Native reasoning: ')}{reasoning.profile}</span>;
    }
    if (reasoning.kind === 'discrete') {
      return <label className="field"><span className="field-label">{text('思考强度', 'Reasoning effort')}</span><select className="select" value={selection?.kind === 'profile' ? selection.profile : ''} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'profile', profile: event.target.value })}>{reasoning.profiles.map(profile => <option key={profile} value={profile}>{profile}</option>)}</select></label>;
    }
    if (reasoning.kind === 'toggle') {
      return <label className="field"><span className="field-label">{text('思考', 'Reasoning')}</span><select className="select" value={selection?.kind === 'toggle' && selection.enabled ? 'on' : 'off'} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'toggle', enabled: event.target.value === 'on' })}><option value="on">{text('开启', 'On')}</option><option value="off">{text('关闭', 'Off')}</option></select></label>;
    }
    return <label className="field"><span className="field-label">{text('思考预算', 'Reasoning budget')}</span><input className="input" type="number" min={reasoning.minimum_tokens} max={reasoning.maximum_tokens} step={reasoning.step_tokens} value={selection?.kind === 'budget' ? selection.tokens : ''} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'budget', tokens: Number(event.target.value) })} /><span className="field-help">{reasoning.minimum_tokens}–{reasoning.maximum_tokens} tokens</span></label>;
  };
  return <div className="worker-choices">
    <p className="field-help" data-codex-shared-scope>{activeCodexMode === 'profile'
      ? text('按需使用 · 仅 Codex CLI。启用后将启动命令粘贴到终端，开启新会话。普通 Codex 和 Desktop 保持原配置。', 'On demand · Codex CLI only. After enabling, paste the launch command into a terminal to start a new session. Ordinary Codex and Desktop keep their settings.')
      : text('设为默认 · 修改同一配置目录下 Codex CLI 和 Desktop 的模型接入。保存后重新启动对应客户端即可生效。', 'Use by default · changes model access for Codex CLI and Desktop sharing this directory. Restart the client after saving to apply the settings.')}</p>
    <fieldset><legend className="field-label">{text('允许的智能路由', 'Allowed smart routes')}</legend>
      {enabledPlans.length === 1 && creatingModelConnection && <p className="field-help">{text('唯一可用路由已选为默认，可直接启用。', 'The only available route is selected as the default. Enable to continue.')}</p>}
      {enabledPlans.map(plan => <label className="check-row agent-route-choice" data-agent-plan-id={plan.agent_plan_id} key={plan.agent_plan_id}><input type="checkbox" checked={editorValues.allowedPlanIds.includes(plan.agent_plan_id)} onChange={() => toggleAllowedPlan(plan.agent_plan_id)} /><div><strong>{plan.desired.display_name}</strong></div></label>)}
      {!enabledPlans.length && <div className="callout"><UiIcon name="route" /><div><span>{text('先创建并启用一条智能路由。', 'Create and enable a smart route first.')}</span></div>{onCreatePlan && <button className="btn" type="button" onClick={onCreatePlan}>{text('创建路由', 'Create route')}</button>}</div>}
      {editorValues.allowedPlanIds.filter(id => !planEnabled(id)).map(id => <label className="check-row agent-route-choice" key={id}><input type="checkbox" checked onChange={() => toggleAllowedPlan(id)} /><span>{planName(id) ?? id} · {text('当前不可用', 'Currently unavailable')}</span></label>)}
      {editorValues.allowedPlanIds.some(id => !planEnabled(id)) && <span className="oc-inline-error">{text('已有路由已停用或不存在，请取消选择后再保存。', 'A previously allowed route is disabled or missing. Remove it before saving.')}</span>}
    </fieldset>
    {showCodexDefault && <>
    <label className="field"><span className="field-label">{text('默认选择', 'Default selection')}</span><select className="select" data-agent-default value={defaultChoiceValue(editorValues.defaultChoice)} onChange={event => selectDefaultChoice(event.target.value)}>{editorValues.nativeModelMode === 'preserve_available' && <option value="native">{text('使用 Codex 当前默认模型名称', 'Use the current Codex default model name')}</option>}{editorValues.fixedModels.map(model => <option key={model.client_model_id} value={`fixed:${model.client_model_id}`}>{model.client_model_id}</option>)}{editorValues.allowedPlanIds.filter(planEnabled).map(id => <option key={id} value={`plan:${id}`}>{planName(id)}</option>)}</select><span className="field-help">{defaultChoice.kind === 'preserve_native'
      ? text('保留当前默认模型名称；请求仍经过 HiRoute。该名称须对应已勾选的路由或已绑定的固定模型。', 'Keep the current default model name; requests still pass through HiRoute. The name must match an enabled route or a bound fixed model.')
      : defaultChoice.kind === 'plan'
        ? editorValues.nativeModelMode === 'preserve_available' ? text('Codex 将默认使用所选智能路由；已证明可用的原生模型名称会继续自动保留。', 'Codex will use the selected smart route by default; proven native model names remain available automatically.') : text('Codex 将默认使用所选智能路由。', 'Codex will use the selected smart route by default.')
        : text('Codex 将默认使用所选固定模型来源。', 'Codex will use the selected fixed model source by default.')}</span>{editorValues.nativeModelMode === 'preserve_available' && defaultChoice.kind === 'preserve_native' && !codexDefaultValid && <span className="oc-inline-error">{text('当前无法读取 Codex 原生默认模型；请修复当前配置后重试。', 'The native Codex default model cannot be read. Repair the current configuration and try again.')}</span>}</label>
    </>}
    {fixedModelRows.length > 0 && <fieldset data-agent-fixed-models><legend className="field-label">{text('已配置的原生模型固定来源', 'Configured native model sources')}</legend>
      {fixedModelRows.map(model => {
        const fixed = editorValues.fixedModels.find(item => item.client_model_id === model.client_model_id);
        const source = fixed ? model.source_options.find(option => option.binding_id === fixed.candidate.binding_id) : undefined;
        return <div className="candidate-row" key={model.client_model_id} data-client-model-id={model.client_model_id}>
          <div><strong>{model.display_name}</strong><small><code>{model.client_model_id}</code></small>{protectedNativeModelIds.has(model.client_model_id) ? <span className="field-help" data-protected-native-model>{text('原生模型绑定由 HiRoute 自动保留；调整路由时无需重新选择。停用模型路由后可重新配置原生来源。', 'HiRoute keeps this native model binding automatically when routes are edited. Disable model routing to change its native source.')}</span> : null}<label className="field"><span className="field-label">{text('固定来源与账号范围', 'Fixed source and account scope')}</span><select className="select" value={fixed?.candidate.binding_id ?? ''} onChange={event => selectFixedSource(model, event.target.value)} disabled={protectedNativeModelIds.has(model.client_model_id)}>{!protectedNativeModelIds.has(model.client_model_id) && <option value="">{text('不通过 HiRoute 固定此名称', 'Do not fix this name through HiRoute')}</option>}{fixed && !source && <option value={fixed.candidate.binding_id} disabled>{protectedNativeModelIds.has(model.client_model_id) ? text('原有绑定已保留，当前列表未显示来源', 'Binding retained; source is not in the current list') : text('原有来源当前不可用', 'Previous source unavailable')}</option>}{model.source_options.map(option => <option key={option.binding_id} value={option.binding_id} disabled={option.state !== 'ready'}>{option.source_label} · {option.account_scope_ref} · {sourceStateLabel(option.state)}</option>)}</select></label>{source && <span className="field-help">{sourceStateLabel(source.state)} · {text('账号范围摘要 ', 'Account scope digest ')}<code>{source.account_scope_digest.slice(0, 18)}…</code></span>}{fixed && source?.state === 'ready' && !protectedNativeModelIds.has(model.client_model_id) && fixedReasoningControl(model, fixed, source)}{!model.source_options.length && !protectedNativeModelIds.has(model.client_model_id) && <span className="field-help">{text('此原生名称尚无独立来源/账号绑定；元数据不会被当作可调用证明。', 'This native name has no independent source/account binding; metadata is not treated as callable proof.')}</span>}</div>
        </div>;
      })}
    </fieldset>}
    <Disclosure label={text('高级设置', 'Advanced settings')} language={language}>
      {selected.codex_access && <CodexAccessSettings access={selected.codex_access} mode={activeCodexMode} language={language} disabled={disabled} onMode={onCodexMode} />}
    <fieldset data-agent-native-mode><legend className="field-label">{text('Codex 可用模型', 'Models available in Codex')}</legend>
      <label className="check-row"><input type="radio" name="codex-native-mode" value="hiroute_only" checked={editorValues.nativeModelMode === 'hiroute_only'} onChange={() => selectNativeModelMode('hiroute_only')} /><div><strong>{text('只使用已配置的 HiRoute 模型', 'Use configured HiRoute models only')}</strong><small>{text('无需接入原 Codex 账号；关闭时恢复原配置。', 'No original Codex account connection required; disabling restores the original configuration.')}</small></div></label>
      <label className="check-row"><input type="radio" name="codex-native-mode" value="preserve_available" checked={editorValues.nativeModelMode === 'preserve_available'} onChange={() => selectNativeModelMode('preserve_available')} /><div><strong>{text('同时保留 Codex 原有模型', 'Also keep native Codex models')}</strong><small>{text('先在“模型”接入原订阅账号或 API 来源；只保留能证明同账号可调用的模型。缓存目录不是账号权限。', 'Connect the original subscription account or API source under Models first. Only proven same-account models are kept; the cache is not entitlement.')}</small></div></label>
    </fieldset>
    <div className="callout" data-codex-shared-scope><UiIcon name="info" /><div><strong>{activeCodexMode === 'profile' ? text('独立 CLI 配置', 'Separate CLI configuration') : text('一套共享配置', 'One shared configuration')}</strong><p>{activeCodexMode === 'profile'
      ? text('本次保存只作用于指定 profile 的 Codex CLI。Desktop 继续使用原来的默认配置。', 'This save applies only to Codex CLI using the selected profile. Desktop keeps its original default configuration.')
      : text('保存一次即作用于共享此配置作用域与 CODEX_HOME 的 Codex CLI 和 Desktop。', 'One save applies to Codex CLI and Desktop when they share this configuration scope and CODEX_HOME.')}</p>{codexSurfaces.map(({surface, detected, applicable}) => {
      return <span className="field-help" key={surface} data-agent-surface-fact={surface} data-agent-surface-detected={detected ? 'true' : 'false'} data-agent-surface-applicable={applicable ? 'true' : 'false'}>{surfaceName(surface)} · {detected ? text('当前已发现可执行入口', 'currently detected as runnable') : text('当前未发现可执行入口', 'not currently detected as runnable')} · {applicable ? text('适用当前模式', 'applies to this mode') : text('不适用当前模式', 'does not apply to this mode')}</span>;
    })}</div></div>
      <p className="field-help" data-agent-plan-compatibility>{text('保存时会核对路由的 Codex Responses 能力。不同入口的会话历史可能不同。', 'Saving checks the routes’ Codex Responses capabilities. History may differ between entry points.')}</p>
    </Disclosure>
    {modelFormInvalid && <span className="oc-inline-error">{text('请选择可用路由或固定模型，并修正不可用项。', 'Choose an available route or fixed model and resolve unavailable selections.')}</span>}
  </div>;
}
