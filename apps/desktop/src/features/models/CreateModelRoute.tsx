import { useState } from 'react';
import { Dialog, UiIcon } from '../../ui';
import type { Plan } from '../../plan-editor';
import type { ManagedModel, ManagementSnapshot } from './types';
import { matchingModelRoutes, matchingModelSources, type ModelRouteRequest } from './model-route';

export function CreateModelRoute({ model, snapshot, plans, language, disabled, onCreate, onOpenPlan }: {
  model: ManagedModel; snapshot: ManagementSnapshot; plans: Plan[]; language: 'zh' | 'en'; disabled: boolean;
  onCreate(request: ModelRouteRequest): void; onOpenPlan?(planId: string): void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [choice, setChoice] = useState<{ others: ReturnType<typeof matchingModelSources>; routes: Plan[]; planId?: string } | null>(null);
  const selected = choice?.routes.find(plan => plan.agent_plan_id === choice.planId);
  const canAppend = !selected || selected.desired.mode === 'fixed_model';
  function create() {
    const others = matchingModelSources(snapshot, model);
    const routes = matchingModelRoutes(model.upstream_model_id, plans);
    if (!others.length && !routes.length) {
      onCreate({ modelId: model.upstream_model_id, bindingIds: [model.binding_id] });
    } else setChoice({ others, routes, planId: routes[0]?.agent_plan_id });
  }
  function continueWith(includeOthers: boolean) {
    if (selected && !canAppend) { onOpenPlan?.(selected.agent_plan_id); setChoice(null); return; }
    const request = { modelId: model.upstream_model_id, bindingIds: [model.binding_id, ...(includeOthers ? choice?.others.map(item => item.model.binding_id) ?? [] : [])], plan: selected };
    setChoice(null);
    onCreate(request);
  }
  return <div className="model-route-action">
    <button className="btn btn-primary" type="button" disabled={disabled} onClick={create}><UiIcon name="plus" />{text('创建智能路由', 'Create routing')}</button>
    <Dialog open={!!choice} title={selected ? text('已有同名路由', 'A matching route exists') : text('发现同名模型', 'Matching models found')}
      closeLabel={text('关闭创建路由', 'Close route creation')} onClose={() => setChoice(null)}
      footer={<><button className="btn" type="button" onClick={() => setChoice(null)}>{text('取消', 'Cancel')}</button>
        {canAppend && !!choice?.others.length && <button className="btn" type="button" disabled={disabled} onClick={() => continueWith(false)}>{selected ? text('仅加入当前模型', 'Add current model only') : text('仅用当前模型', 'Use current model only')}</button>}
        <button className="btn btn-primary" type="button" disabled={disabled || (!canAppend && !onOpenPlan)} onClick={() => continueWith(true)}>{!canAppend ? text('查看路由', 'View route') : choice?.others.length ? text('一起加入候选', 'Add together as candidates') : text('编辑已有路由', 'Edit existing route')}</button></>}>
      <p><strong>{model.upstream_model_id}</strong></p>
      {!!choice?.routes.length && <>
        <p>{text('可加入已有路由，保存后生效。', 'Add to an existing route, then save to apply.')}</p>
        <label className="field"><span className="field-label">{text('已有路由', 'Existing route')}</span><select className="input" value={choice.planId} onChange={event => setChoice({ ...choice, planId: event.target.value })}>{choice.routes.map(plan => <option key={plan.agent_plan_id} value={plan.agent_plan_id}>{plan.desired.display_name} · {plan.model_alias}</option>)}</select></label>
      </>}
      {canAppend && !!choice?.others.length && <>
        <p>{text('其他接入点也有这个模型，是否一起加入候选？', 'Other connections offer this model. Add them as candidates too?')}</p>
        <ul className="model-route-matches">{choice.others.map(item => <li key={item.model.binding_id}>{item.source.display_name}</li>)}</ul>
        <p className="field-help">{selected ? text('保留已有候选顺序，新模型排在后面。', 'Existing candidate order is retained; new models are appended.') : text('当前模型优先，其他接入作为备用。', 'The current model comes first; other connections provide fallback.')}</p>
      </>}
      {!canAppend && <p className="field-help">{text('这条路由使用其他模式，请在配置页调整候选。', 'This route uses another mode. Adjust its candidates in the configuration page.')}</p>}
    </Dialog>
  </div>;
}
