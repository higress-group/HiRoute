import React from 'react';
import { UiIcon } from '../../ui';
import { agentDisplayName } from './ecosystems';
import type { AgentModelEditorProps } from './AgentModelEditor';

/** Shared Plan selection for ecosystems that add an independent native provider. */
export function AdditionalModelEditor({ agent, values, plans, language, disabled, invalid, onChange, onEdited, onCreatePlan }: AgentModelEditorProps) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const name = agentDisplayName(agent.agent_id, language);
  const enabledPlans = plans.filter(plan => plan.head.status === 'enabled');
  const unavailable = values.allowedPlanIds.filter(id => !enabledPlans.some(plan => plan.agent_plan_id === id));
  function toggle(planId: string) {
    onChange(current => ({ ...current, allowedPlanIds: current.allowedPlanIds.includes(planId)
      ? current.allowedPlanIds.filter(id => id !== planId) : [...current.allowedPlanIds, planId] }));
    onEdited();
  }
  return <div className="worker-choices" data-additional-routes={name} data-qoder-additional-routes={name === 'Qoder' ? true : undefined}>
    <p>{text(`将所选智能路由添加到 ${name} 的模型选择中。原有模型、提供方、登录和当前默认模型保持不变。`, `Add selected smart routes to ${name}’s model choices. Existing models, providers, sign-in and the current default model stay unchanged.`)}</p>
    <fieldset disabled={disabled} className="worker-choices">
      <legend className="field-label">{text(`添加智能路由`, `Add smart routes`)}</legend>
      {enabledPlans.map(plan => <label className="check-row" key={plan.agent_plan_id} data-agent-plan-id={plan.agent_plan_id}>
        <input type="checkbox" checked={values.allowedPlanIds.includes(plan.agent_plan_id)} onChange={() => toggle(plan.agent_plan_id)} />
        <span>{plan.desired.display_name}</span>
      </label>)}
      {unavailable.map(id => <label className="check-row" key={id} data-agent-plan-id={id}>
        <input type="checkbox" checked onChange={() => toggle(id)} />
        <span>{plans.find(plan => plan.agent_plan_id === id)?.desired.display_name ?? id} · {text(`当前不可用，可取消选择`, `Currently unavailable; deselect to remove`)}</span>
      </label>)}
    </fieldset>
    <p className="field-help" data-agent-activation>{text(`保存后重新启动 ${name}，通过 /model 明确选用已添加的 HiRoute 路由。未选择时继续使用原来的模型。`, `Restart ${name} after saving, then explicitly select an added HiRoute route with /model. Otherwise it keeps using your previous model.`)}</p>
    <p className="field-help">{text(`在 HiRoute 中调整或移除这些路由。如果已将某条路由设为 ${name} 默认模型，移除前请先在 ${name} 中切换到其他模型。`, `Adjust or remove these routes in HiRoute. If one is now your ${name} default, switch to another model in ${name} before removing it.`)}</p>
    {invalid && <span className="oc-inline-error">{text(`至少选择一条已启用路由，并取消不可用的选择。`, `Select at least one enabled route and remove unavailable selections.`)}</span>}
    {!enabledPlans.length && <div className="callout"><UiIcon name="route" /><div>{text(`先创建并启用一条智能路由。任务协作仍可独立启用。`, `Create and enable a smart route first. Task collaboration can still be enabled independently.`)}</div>{onCreatePlan && <button className="btn" type="button" onClick={onCreatePlan}>{text(`创建路由`, `Create route`)}</button>}</div>}
  </div>;
}
