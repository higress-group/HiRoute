import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import type { Draft, Editor, Options, Plan } from '../../plan-editor';
import type { DesktopSnapshot } from '../../product/home-projections';
import { routingEditorEntries } from '../../routing-editor-entries';
import { replacementSlots, replaceRouteModel } from './replacement-route';
import { sameConnection, type RecommendationConnections, type RecommendedConnection } from './recommendation-connection';

export type RecommendationReplacement = { plan: Plan; draft?: Draft; editor: Editor };

export function ReplacementRoute({ connection, language, mutable, readConnections, onConnectionChanged, onBack, onSave, onBusy }: {
  connection: RecommendedConnection;
  language: 'zh' | 'en';
  mutable: boolean;
  readConnections(): Promise<RecommendationConnections>;
  onConnectionChanged(current?: RecommendationConnections): void;
  onBack(): void;
  onSave(change: RecommendationReplacement): Promise<boolean>;
  onBusy(value: boolean): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [catalog, setCatalog] = useState<DesktopSnapshot['catalog'] | null>(null);
  const [routeId, setRouteId] = useState('');
  const [route, setRoute] = useState<(RecommendationReplacement & { options: Options }) | null>(null);
  const [slotId, setSlotId] = useState('');
  const [reviewing, setReviewing] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [retry, setRetry] = useState(0);
  const reviewHeading = useRef<HTMLHeadingElement>(null);
  useEffect(() => { if (reviewing) reviewHeading.current?.focus(); }, [reviewing]);
  const entries = catalog ? routingEditorEntries(catalog.plans, catalog.drafts).filter(entry => entry.plan && !entry.staleDraft) : [];
  const selected = entries.find(entry => entry.plan?.agent_plan_id === routeId);

  useEffect(() => {
    let current = true;
    void invoke<DesktopSnapshot>('desktop_snapshot').then(snapshot => {
      if (snapshot.catalog_error) throw Error('ROUTING_UNAVAILABLE');
      if (current) setCatalog(snapshot.catalog);
    }).catch(() => {
      if (current) setError(t('暂时无法读取智能路由。', 'Smart routes could not be read.'));
    });
    return () => { current = false; };
  }, [retry]);

  useEffect(() => {
    const plan = selected?.plan;
    if (!plan) return;
    const draft = selected.draft;
    let current = true;
    void invoke<Options>('plan_editor_options', { input: { authoring: { plan_id: plan.agent_plan_id }, include_unavailable: true } }).then(options => {
      if (!current) return;
      if (options.authoring?.plan_id !== plan.agent_plan_id || options.authoring.expected_head_revision !== plan.head.head_revision) {
        setError(t('路由已更新，请刷新后重新选择。', 'The route changed. Refresh and select it again.'));
        return;
      }
      if (!options.candidates.some(candidate => candidate.binding_id === connection.bindingId && candidate.routable)) {
        onConnectionChanged();
        return;
      }
      setRoute({ plan, draft, editor: draft?.editor ?? options.authoring.editor, options });
    }).catch(() => {
      if (current) setError(t('暂时无法读取这个路由的模型位置。', 'The model positions for this route could not be read.'));
    });
    return () => { current = false; };
  }, [selected?.plan, selected?.draft, connection.bindingId]);

  const candidate = route?.options.candidates.find(value => value.binding_id === connection.bindingId);
  const slots = route && candidate ? replacementSlots(route.editor, route.options, candidate, language) : [];
  const slot = slots.find(value => value.id === slotId);
  function refresh() {
    setError(''); setCatalog(null); setRouteId(''); setRoute(null); setSlotId(''); setReviewing(false);
    setRetry(value => value + 1);
  }
  async function saveDraft() {
    if (busy || !route || !candidate || !slot || !mutable) return;
    setBusy(true); onBusy(true); setError('');
    try {
      const fresh = await readConnections();
      if (!fresh.ready.some(value => sameConnection(value, connection))) {
        onConnectionChanged(fresh);
        return;
      }
      const accepted = await onSave({ plan: route.plan, draft: route.draft, editor: replaceRouteModel(route.editor, slot, candidate, route.options) });
      if (!accepted) setError(t('当前无法打开路由编辑器，请重试。', 'The route editor could not be opened. Retry.'));
    } catch {
      setError(t('暂时无法确认接入和草稿，请重试。', 'The connection and draft could not be confirmed. Retry.'));
    } finally { onBusy(false); setBusy(false); }
  }

  return <section className="recommendation-step" aria-busy={busy}>
    {reviewing && candidate && slot && route ? <>
      <h3 tabIndex={-1} ref={reviewHeading}>{t('预览模型替换', 'Preview model replacement')}</h3>
      <dl className="recommendation-review">
        <div><dt>{t('路由', 'Route')}</dt><dd>{route.editor.display_name}</dd></div>
        <div><dt>{t('位置', 'Position')}</dt><dd>{slot.label}</dd></div>
        <div><dt>{t('原模型', 'Current model')}</dt><dd>{slot.oldName}</dd></div>
        <div><dt>{t('替换为', 'Replace with')}</dt><dd>{candidate.display_name} · {connection.sourceName}</dd></div>
      </dl>
      <p className="field-help">{t('保存后，在智能路由中发布更改才会生效。', 'After saving, publish the change in Smart routing to make it active.')}</p>
    </> : <>
      <p className="field-help">{t('已确认可用的接入', 'Available connection')} · {connection.sourceName}</p>
      {catalog && entries.length > 0 && <label className="field">
        <span className="field-label">{t('路由', 'Route')}</span>
        <select className="select" value={routeId} onChange={event => {
          setRouteId(event.target.value); setRoute(null); setSlotId(''); setError('');
        }}>
          <option value="">{t('选择路由', 'Choose a route')}</option>
          {entries.map(entry => <option key={entry.key} value={entry.plan!.agent_plan_id}>{(entry.draft?.editor.display_name ?? entry.plan!.desired.display_name) + (entry.draft ? t('（有草稿）', ' (has draft)') : '')}</option>)}
        </select>
      </label>}
      {catalog && !entries.length && <p role="status">{t('暂无可替换的已有路由。', 'There are no existing routes to update.')}</p>}
      {!catalog && !error && <p role="status">{t('正在读取路由…', 'Reading routes…')}</p>}
      {routeId && !route && !error && <p role="status">{t('正在读取模型位置…', 'Reading model positions…')}</p>}
      {route && (slots.length ? <>
        <label className="field">
          <span className="field-label">{t('模型位置', 'Model position')}</span>
          <select className="select" value={slotId} onChange={event => setSlotId(event.target.value)}>
            <option value="">{t('选择要替换的位置', 'Choose a position to replace')}</option>
            {slots.map(value => <option key={value.id} value={value.id} disabled={!!value.reason}>{value.label} · {value.oldName}{value.reason && ` (${value.reason})`}</option>)}
          </select>
        </label>
        {route.draft && <p className="field-help">{t('这次替换会写入当前路由草稿。', 'This replacement will be saved in the route’s current draft.')}</p>}
        {slots.every(value => value.reason) && <p role="status">{t('当前接入与这个路由的模型位置不兼容，请选择其他路由。', 'This connection cannot replace a model in this route. Choose another route.')}</p>}
      </> : <p role="status">{t('这个路由没有已配置的模型位置。', 'This route has no configured model positions.')}</p>)}
    </>}
    {error && <p role="alert" className="oc-inline-error">{error}</p>}
    <div className="actions">
      <button type="button" className="btn" disabled={busy} onClick={() => {
        if (reviewing) { setReviewing(false); setError(''); }
        else onBack();
      }}>{t('返回', 'Back')}</button>
      {error && !reviewing && <button type="button" className="btn" onClick={refresh}>{t('刷新路由', 'Refresh routes')}</button>}
      {reviewing ? <button type="button" className="btn btn-primary" disabled={busy || !mutable} onClick={() => void saveDraft()}>{t('保存草稿', 'Save draft')}</button>
        : <button type="button" className="btn btn-primary" disabled={!slot || !!slot.reason || !mutable} onClick={() => setReviewing(true)}>{t('预览替换', 'Preview replacement')}</button>}
    </div>
  </section>;
}
