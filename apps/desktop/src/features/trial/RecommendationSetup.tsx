import { useEffect, useRef, useState } from 'react';
import { Dialog } from '../../ui';
import { requestEditorReplacement } from '../../ui/discard-guard';
import type { AvailableModel } from '../model-feed/types';
import type { OperationReference } from '../model-connections/types';
import { TrialConnectModel, type TrialConnectionState } from './TrialConnectModel';
import { TrialReuseConnection } from './TrialReuseConnection';
import { ReplacementRoute, type RecommendationReplacement } from './ReplacementRoute';
import {
  readRecommendationConnections, readRecommendationRequestModelIds, referenceModelKey, sameConnection,
  type RecommendationConnections, type RecommendedConnection,
} from './recommendation-connection';

export function RecommendationSetup({ model, language, mutable, hasRoutes, onTry, onSave, onClose, onOperation, onConnectionsChanged }: {
  model: AvailableModel;
  language: 'zh' | 'en';
  mutable: boolean;
  hasRoutes: boolean;
  onTry(intent: { modelId: string }): Promise<boolean>;
  onSave(change: RecommendationReplacement): Promise<boolean>;
  onClose(): void;
  onOperation(operation: OperationReference): void;
  onConnectionsChanged(): void;
}) {
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  const reference = referenceModelKey(model);
  const [replacing, setReplacing] = useState(false);
  const [data, setData] = useState<RecommendationConnections>();
  const [requestModelIds, setRequestModelIds] = useState<ReadonlyMap<string, string>>();
  const [confirmed, setConfirmed] = useState<RecommendedConnection>();
  const [choice, setChoice] = useState('');
  const [creating, setCreating] = useState(false);
  const [creationHidden, setCreationHidden] = useState(false);
  const [reuse, setReuse] = useState<{ sourceId: string; requestId?: string; hidden?: boolean }>();
  const [connectionState, setConnectionState] = useState<TrialConnectionState>('editing');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const content = useRef<HTMLDivElement>(null);
  const fresh = useRef<RecommendationConnections>(undefined);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  useEffect(() => {
    const container = content.current;
    if (!container || container.contains(document.activeElement)) return;
    const first = Array.from(container.querySelectorAll<HTMLElement>('select:not(:disabled), button:not(:disabled)'))
      .find(element => element.getClientRects().length > 0);
    (first ?? container).focus();
  }, [replacing, creating, confirmed, reuse, creationHidden, busy]);

  const read = () => readRecommendationConnections(model, requestModelIds);
  async function loadConnections() {
    setBusy(true); setError(''); setData(undefined);
    try {
      const ids = reference ? await readRecommendationRequestModelIds(reference) : undefined;
      const current = await readRecommendationConnections(model, ids);
      if (!live.current) return;
      setRequestModelIds(ids); setData(current); setChoice(current.ready.length === 1 ? current.ready[0].bindingId : '');
    } catch {
      if (live.current) setError(t('暂时无法读取接入，请重试。', 'Connections could not be read. Retry.'));
    } finally { if (live.current) setBusy(false); }
  }

  async function start(next: 'trial' | 'replace') {
    if (!mutable || busy) return;
    if (next === 'trial') {
      if (await onTry({ modelId: model.model_configuration_id })) onClose();
      return;
    }
    setReplacing(true);
    await loadConnections();
  }

  function finish(connection: RecommendedConnection) {
    if (!live.current) return false;
    setCreating(false); setReuse(undefined); setConnectionState('editing'); setConfirmed(connection); setError('');
    return true;
  }

  function connectionChanged(current?: RecommendationConnections) {
    if (current) setData(current);
    setConfirmed(undefined); setChoice('');
    setError(t('所选接入已变化或暂不可用，请重新确认。', 'The selected connection changed or is unavailable. Confirm a connection again.'));
  }

  async function confirmExisting() {
    const selected = data?.ready.find(value => value.bindingId === choice);
    if (!selected || busy || !mutable) return;
    setBusy(true); setError('');
    try {
      const current = await read();
      if (!live.current) return;
      setData(current);
      const stillAvailable = current.ready.find(value => sameConnection(value, selected));
      if (stillAvailable) finish(stillAvailable);
      else connectionChanged(current);
    } catch {
      if (live.current) setError(t('暂时无法确认所选接入，请重试。', 'The selected connection could not be confirmed. Retry.'));
    } finally { if (live.current) setBusy(false); }
  }
  function backFromCreation(recoverable: boolean) {
    if (recoverable && connectionState === 'uncertain') {
      setCreationHidden(true);
      return;
    }
    setCreating(false); setConnectionState('editing'); void loadConnections();
  }
  async function close() {
    if (busy || connectionState === 'saving' || !await requestEditorReplacement('models')) return;
    onClose();
  }
  const backLabel = t('返回接入方式', 'Back to connection options');
  const title = confirmed ? t('替换已有路由里的模型', 'Replace a model in an existing route')
    : replacing ? t(`接入 ${model.display_name}`, `Connect ${model.display_name}`)
      : t(`配置 ${model.display_name}`, `Configure ${model.display_name}`);

  const reuseDialog = reuse && <TrialReuseConnection
    model={model} sourceId={reuse.sourceId} requestModelId={reuse.requestId} requestModelIds={requestModelIds} visible={!reuse.hidden} language={language} mutable={mutable} backLabel={backLabel}
    onOperation={onOperation} onClose={recoverable => {
      if (recoverable) setReuse({ ...reuse, hidden: true });
      else { setReuse(undefined); void loadConnections(); }
    }}
    onUse={async selected => {
      const current = await read();
      const ready = current.ready.find(binding => binding.bindingId === selected.bindingId && binding.sourceId === selected.sourceId
        && binding.revision === selected.revision && binding.modelRef === selected.modelRef);
      if (!ready) return false;
      setData(current); onConnectionsChanged();
      return finish(ready);
    }}
  />;

  return <>{reuseDialog}{(!reuse || reuse.hidden) && <Dialog open title={title} closeLabel={t('关闭', 'Close')} closeDisabled={busy || connectionState === 'saving'} onClose={() => void close()} className="trial-dialog recommendation-dialog">
    <div ref={content} tabIndex={-1}>
      {reuse?.hidden ? <section className="recommendation-step">
        <p className="field-help">{t('请先核对这次接入的保存结果。', 'Confirm the result of this connection save first.')}</p>
        <button type="button" className="btn" onClick={() => setReuse({ ...reuse, hidden: false })}>{t('查看接入结果', 'Review connection result')}</button>
      </section> : confirmed ? <ReplacementRoute connection={confirmed} language={language} mutable={mutable} readConnections={read}
        onConnectionChanged={connectionChanged} onBusy={setBusy} onBack={() => { setConfirmed(undefined); setChoice(confirmed.bindingId); }}
        onSave={async change => {
          const accepted = await onSave(change);
          if (accepted) onClose();
          return accepted;
        }}
      /> : !replacing ? <div className="trial-choice-list">
        <button type="button" className="trial-choice" disabled={!mutable} onClick={() => void start('trial')}>
          <strong>{t('新建试用路由', 'Create a trial route')}</strong>
          <span>{t('走试用流程，选择主力模型与 Agent。', 'Use the trial setup to choose a primary model and Agent.')}</span>
        </button>
        <button type="button" className="trial-choice" disabled={!mutable || !hasRoutes} onClick={() => void start('replace')}>
          <strong>{t('替换已有路由里的模型', 'Replace a model in an existing route')}</strong>
          <span>{hasRoutes ? t('确认接入后，选择路由和替换位置。', 'Confirm a connection, then choose a route and position.') : t('暂无已有路由。', 'There are no existing routes.')}</span>
        </button>
      </div> : creating && data ? <>
        {creationHidden && <section className="recommendation-step">
          <p className="field-help">{t('请先核对这次接入的保存结果。', 'Confirm the result of this connection save first.')}</p>
          <button type="button" className="btn" onClick={() => setCreationHidden(false)}>{t('查看接入结果', 'Review connection result')}</button>
        </section>}
        <section className="recommendation-step" hidden={creationHidden}>
          {error && <p role="alert" className="oc-inline-error">{error}</p>}
          <TrialConnectModel model={model} language={language} mutable={mutable} management={data.setup.management} requestModelIds={requestModelIds} backLabel={backLabel}
            onResolve={async () => {
              const current = await read();
              if (live.current) { fresh.current = current; setData(current); return current.setup; }
            }}
            onReceipt={onConnectionsChanged} onOperation={onOperation} onSelectionIntent={() => 0}
            onSaved={(receipt, selection) => {
              const ready = fresh.current?.ready.find(value => value.bindingId === selection.bindingId && value.sourceId === receipt.source_id);
              if (ready) return finish(ready);
              setError(t('该接入已保存，请选择能确认是推荐模型的可用接入。', 'This connection was saved. Choose an available connection confirmed for the recommended model.'));
              return false;
            }}
            onBack={backFromCreation} onStateChange={setConnectionState} />
        </section>
      </> : <section className="recommendation-step" aria-busy={busy}>
        <p className="field-help">{t('确认接入可用后，再选择路由和模型位置。', 'Confirm an available connection before choosing a route and model position.')}</p>
        {busy && <p role="status">{t('正在确认接入…', 'Checking connections…')}</p>}
        {error && <p role="alert" className="oc-inline-error">{error}</p>}
        {!busy && data && <>
          {data.ready.length > 0 && <label className="field">
            <span className="field-label">{t('可用接入', 'Available connections')}</span>
            <select className="select" value={choice} onChange={event => setChoice(event.target.value)}>
              <option value="">{t('选择接入', 'Choose a connection')}</option>
              {data.ready.map(connection => <option key={connection.bindingId} value={connection.bindingId}>{connection.sourceName}</option>)}
            </select>
          </label>}
          {data.reusable.length > 0 && <label className="field">
            <span className="field-label">{t('复用已有接入', 'Reuse an existing connection')}</span>
            <select className="select" value="" disabled={!mutable} onChange={event => setReuse(data.reusable.find(value => value.sourceId === event.target.value))}>
              <option value="">{t('选择要复用的接入', 'Choose a connection to reuse')}</option>
              {data.reusable.map(connection => <option key={connection.sourceId} value={connection.sourceId}>{connection.sourceName}</option>)}
            </select>
          </label>}
          {requestModelIds && requestModelIds.size === 0 && <p role="status">{t('当前版本暂无支持此模型的接入产品。', 'This version has no connection product for this model.')}</p>}
        </>}
        <div className="actions">
          <button type="button" className="btn" disabled={busy} onClick={() => { setReplacing(false); setError(''); }}>{t('返回', 'Back')}</button>
          {!busy && (!data || error) && <button type="button" className="btn" onClick={() => void loadConnections()}>{t('刷新接入', 'Refresh connections')}</button>}
          {data && (!requestModelIds || requestModelIds.size > 0) && <button type="button" className={data.ready.length ? 'btn' : 'btn btn-primary'} disabled={busy || !mutable} onClick={() => { fresh.current = undefined; setCreationHidden(false); setCreating(true); }}>{t('新建接入', 'Create a connection')}</button>}
          {data && data.ready.length > 0 && <button type="button" className="btn btn-primary" disabled={busy || !choice || !mutable} onClick={() => void confirmExisting()}>{t('使用所选接入', 'Use selected connection')}</button>}
        </div>
      </section>}
    </div>
  </Dialog>}</>;
}
