import React from 'react';
import { AgentProtocolChoice } from '../AgentProtocolChoice';
import { UiIcon } from '../../ui';
import type { AgentModelEditorProps } from './AgentModelEditor';
import './additional-model-editor.css';

export function HermesModelEditor({ agent, values, plans, language, disabled, invalid, onChange, onEdited, onCreatePlan }: AgentModelEditorProps) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const enabled = plans.filter(plan => plan.head.status === 'enabled');
  const selected = plans.find(plan => plan.agent_plan_id === values.hermesPlanId);
  return <div className="additional-model-editor" data-hermes-main-routing>
    <p>{text('将 Hermes 主对话和工具循环接到一条智能路由。不同任务的模型选择由这条路由管理。', 'Connect the Hermes main conversation and tool loop to one smart route. That route chooses models for different tasks.')}</p>
    <label className="field"><span className="field-label">{text('Hermes 配置目录', 'Hermes configuration directory')}</span><input className="input" readOnly value={agent.native_target?.config_root ?? ''} /></label>
    <label className="field"><span className="field-label">{text('智能路由', 'Smart route')}</span><select className="input" disabled={disabled} value={values.hermesPlanId} onChange={event => { onChange(current => ({ ...current, hermesPlanId: event.target.value })); onEdited(); }}>
      <option value="">{text('选择已启用的路由', 'Select an enabled route')}</option>
      {enabled.map(plan => <option key={plan.agent_plan_id} value={plan.agent_plan_id}>{plan.desired.display_name}</option>)}
      {values.hermesPlanId && !enabled.some(plan => plan.agent_plan_id === values.hermesPlanId) && <option value={values.hermesPlanId}>{selected?.desired.display_name ?? values.hermesPlanId} · {text('当前不可用', 'Currently unavailable')}</option>}
    </select></label>
    {selected && <AgentProtocolChoice allowChatCompletions name={selected.desired.display_name} language={language} value={values.hermesProtocol} advice={selected.protocol_advice} onChange={protocol => { onChange(current => ({ ...current, hermesProtocol: protocol })); onEdited(); }} />}
    {values.hermesProtocol === 'chat_completions' && <p className="field-help">{text('建议使用 Hermes v0.21.7 或更新版本，以保留工具续接中的推理内容。旧版本仍可连接，仅在插件或接入接口不兼容时阻止保存。', 'Hermes v0.21.7 or newer is recommended to preserve reasoning across tool calls. Older versions can connect; saving is blocked only for incompatible plugin or connection interfaces.')}</p>}
    <p className="field-help">{text('保存时会自动安装并启用 HiRoute 适配插件。正在运行的 Hermes 需要重新加载插件或开启新会话；文件已安装不代表当前进程已加载。', 'Saving automatically installs and enables the HiRoute adapter. A running Hermes needs a plugin reload or a new session; installed files do not prove the current process has loaded them.')}</p>
    <p className="field-help">{text('压缩、标题、视觉和其他辅助模型仍由 Hermes 管理。HiRoute 管理 Skill 可独立启用与恢复。', 'Hermes retains compression, title, vision and other auxiliary models. The HiRoute management Skill can be prepared and restored independently.')}</p>
    {invalid && <span className="oc-inline-error">{text('请选择一条当前已启用的智能路由。', 'Select one currently enabled smart route.')}</span>}
    {!enabled.length && <div className="callout"><UiIcon name="route" /><div>{text('先创建并启用一条智能路由。HiRoute 管理仍可独立启用。', 'Create and enable a smart route first. HiRoute management can still be prepared independently.')}</div>{onCreatePlan && <button className="btn" type="button" onClick={onCreatePlan}>{text('创建路由', 'Create route')}</button>}</div>}
  </div>;
}
