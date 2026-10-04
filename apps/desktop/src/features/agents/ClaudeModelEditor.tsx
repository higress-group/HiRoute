import React from 'react';
import { Disclosure, UiIcon } from '../../ui';
import type { AgentModelEditorProps } from './AgentModelEditor';
import { commonClaudePlan, sharedClaudePlan } from './editor-state';
import type { AgentClaudePresetMappings } from './types';

export function ClaudeModelEditor({
  values: editorValues, plans, language, onChange: setEditorValues,
  onEdited, onCreatePlan, invalid: modelFormInvalid,
}: AgentModelEditorProps) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const enabledPlans = plans.filter(plan => plan.head.status === 'enabled');
  const planEnabled = (id: string) => enabledPlans.some(plan => plan.agent_plan_id === id);
  const claudePlanIds = Object.values(editorValues.claudePresets)
    .flatMap(choice => choice.kind === 'plan' ? [choice.plan_id] : []);
  const claudeSharedPlan = commonClaudePlan(editorValues.claudePresets);
  function selectClaudePreset(preset: keyof AgentClaudePresetMappings, planId: string) {
    setEditorValues(value => ({
      ...value,
      claudePresets: {
        ...value.claudePresets,
        [preset]: planId ? { kind: 'plan', plan_id: planId } : { kind: 'preserve_native' },
      },
    }));
    onEdited();
  }

  return <div className="worker-choices">
    <label className="field"><span className="field-label">{claudeSharedPlan === null ? text('模型档位路由', 'Preset routes') : text('共用一条路由', 'Use one route for all presets')}</span><select className="select" aria-label={text('Claude Code 路由', 'Claude Code route')} value={claudeSharedPlan ?? ''} onChange={event => { setEditorValues(value => ({ ...value, claudePresets: sharedClaudePlan(event.target.value) })); onEdited(false); }}><option value="" disabled>{claudeSharedPlan === null ? text('已分别配置模型档位', 'Presets configured separately') : text('选择路由', 'Choose a route')}</option>{claudeSharedPlan && !planEnabled(claudeSharedPlan) && <option value={claudeSharedPlan} disabled>{text('原有路由当前不可用', 'Previous route unavailable')}</option>}{enabledPlans.map(plan => <option key={plan.agent_plan_id} value={plan.agent_plan_id}>{plan.desired.display_name}</option>)}</select><span className="field-help">{claudeSharedPlan === null
      ? text('当前按档位分别配置。选择同一条路由可统一 Opus、Sonnet、Haiku；不会修改 Claude 当前默认模型。', 'Presets are configured separately. Choose one route to unify Opus, Sonnet and Haiku; the current Claude default model is unchanged.')
      : text('应用于 Opus、Sonnet、Haiku；高级设置可分别配置。不会修改 Claude 当前默认模型。', 'Applies to Opus, Sonnet and Haiku; configure them separately under Advanced settings. The current Claude default model is unchanged.')}</span></label>
    {editorValues.fixedModels.length > 0 && <p className="field-help">{text('保留已有固定模型：', 'Existing fixed models retained: ')}{editorValues.fixedModels.map(model => model.client_model_id).join(' · ')}</p>}
    <p className="field-help" data-agent-activation>{text('保存后，重新启动 Claude Code 即可加载配置。', 'Restart Claude Code after saving to load the settings.')}</p>
    <Disclosure label={text('高级设置', 'Advanced settings')} language={language} defaultOpen={claudeSharedPlan === null || claudePlanIds.some(id => !planEnabled(id))}>
      <p className="field-help">{text('普通 claude 入口使用这些映射。账号 Default 仍需真实调用验证，可显式选择已映射的模型档位。', 'The ordinary claude entry uses these mappings. Account Default needs a live call to verify; explicitly choose a mapped preset.')}</p>
    {(['opus', 'sonnet', 'haiku'] as const).map(preset => {
      const choice = editorValues.claudePresets[preset];
      const selectedPlan = choice.kind === 'plan' ? choice.plan_id : '';
      return <label className="field" key={preset}><span className="field-label">{preset[0].toUpperCase() + preset.slice(1)}</span><select className="select" value={selectedPlan} onChange={event => selectClaudePreset(preset, event.target.value)}><option value="">{text('保留原生值或缺省关系', 'Preserve native value or default relationship')}</option>{selectedPlan && !planEnabled(selectedPlan) && <option value={selectedPlan} disabled>{text('原有路由当前不可用', 'Previous route unavailable')}</option>}{enabledPlans.map(plan => <option key={plan.agent_plan_id} value={plan.agent_plan_id}>{plan.desired.display_name}</option>)}</select></label>;
    })}
    </Disclosure>
    {modelFormInvalid && <span className="oc-inline-error">{text('至少将一个预设映射到可用智能路由，或保留已有固定模型。', 'Map at least one preset to an enabled smart route, or retain an existing fixed model.')}</span>}
    {!enabledPlans.length && <div className="callout"><UiIcon name="route" /><div><span>{text('先创建并启用一条智能路由。', 'Create and enable a smart route first.')}</span></div>{onCreatePlan && <button className="btn" type="button" onClick={onCreatePlan}>{text('创建路由', 'Create route')}</button>}</div>}
  </div>;
}
