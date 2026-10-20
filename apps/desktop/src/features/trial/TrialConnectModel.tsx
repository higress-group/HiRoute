import { useMemo, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { ModelConnectionForm } from '../model-connections/ModelConnectionForm';
import { modelConnectionBackend } from '../model-connections/backend';
import { newModelConnectionDraft } from '../model-connections/new-draft';
import type { ComputeSaveResult, ModelConnectionBackend, OperationReference } from '../model-connections/types';
import type { ManagementSnapshot } from '../models/types';
import type { AvailableModel, ModelAvailabilityResult } from '../model-feed/types';

export function TrialConnectModel({ model, language, management, mutable, onSaved, onBack, onOperation }: {
  model: AvailableModel; language: 'zh' | 'en'; management: ManagementSnapshot; mutable: boolean;
  onSaved(result: ComputeSaveResult, model: AvailableModel): void; onBack(): void; onOperation(operation: OperationReference): void;
}) {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const [draft] = useState(() => newModelConnectionDraft('known'));
  const [saved, setSaved] = useState<ComputeSaveResult | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const uncertainRef = useRef(false);
  const accepted = useRef<OperationReference | null>(null);
  const observed = useRef(new Map<string, Set<string>>());
  const backend = useMemo<ModelConnectionBackend>(() => {
    const owner = modelConnectionBackend(language);
    return {
      ...owner,
      listConnectionOptions: async () => {
        const options = await owner.listConnectionOptions();
        return { ...options, options: options.options.filter(option => option.model_configuration_ids.includes(model.model_configuration_id)) };
      },
      checkRegisteredModelConnection: async request => {
        const result = await owner.checkRegisteredModelConnection(request);
        observed.current.set(result.candidate.candidate.candidate_ref, new Set(result.candidate.models.filter(m => m.selectable && (result.observed_model_ids?.includes(m.upstream_model_id) || (result.inference === 'verified' && result.inference_model_id === m.upstream_model_id))).map(m => m.model_ref)));
        return result;
      },
      // This flow is for the selected registered identity, with the same protected
      // input and save contract as Models. Custom connections stay in Models.
      checkModelConnection: async () => { throw new Error('TRIAL_REQUIRES_REGISTERED_MODEL'); },
    };
  }, [language, model.model_configuration_id]);

  async function confirm(result?: ComputeSaveResult) {
    if (busy) return;
    setBusy(true); setError('');
    try {
      const receipt = result ?? saved ?? (accepted.current ? await backend.getComputeSaveResult(accepted.current) : null);
      if (!receipt || receipt.disposition !== 'saved') { setUncertain(true); setError(text('保存结果仍未确认，请稍后刷新。', 'The save result is still unconfirmed. Refresh shortly.')); return; }
      setSaved(receipt);
      const [current, availability] = await Promise.all([
        invoke<ManagementSnapshot>('compute_management_snapshot'), invoke<ModelAvailabilityResult>('model_availability'),
      ]);
      const candidate = receipt.candidate?.candidate_ref;
      const checked = candidate ? observed.current.get(candidate) : undefined;
      const source = current.sources.find(s => s.source_id === receipt.source_id);
      const target = availability.models.find(m => m.model_configuration_id === model.model_configuration_id);
      const binding = source?.models.find(m => m.catalog_configuration_id === model.model_configuration_id
        && receipt.bindings.some(b => b.binding_id === m.binding_id && checked?.has(b.model_ref))
        && target?.binding_ids.includes(m.binding_id));
      if (!target || !binding) { setError(text('接入已保存，但尚未确认所选模型可用。可返回核对已保存接入，不会重复创建。', 'The connection was saved, but the selected model is not confirmed available. Return to review the saved connection; it will not be created again.')); return; }
      onSaved(receipt, target);
    } catch { setError(text('暂时无法核实保存结果。已完成的保存会保留，刷新不会重复提交。', 'The save result could not be verified. Completed saves are retained; refresh does not resubmit.')); }
    finally { setBusy(false); }
  }
  if (saved || uncertain) return <section className="trial-recovery"><h3>{saved ? text('接入已保存', 'Connection saved') : text('正在核实接入结果', 'Verifying connection result')}</h3><p>{error || text('请刷新并核实所选模型。', 'Refresh to verify the selected model.')}</p><div className="actions"><button className="btn" disabled={busy} onClick={() => void confirm()}>{text('刷新结果', 'Refresh result')}</button><button className="btn" disabled={busy} onClick={onBack}>{text('返回试用', 'Back to trial')}</button></div></section>;
  return <section className="trial-inline-connection"><h3>{text('接入', 'Connect')} {model.display_name}</h3>{error && <p role="alert">{error}</p>}<ModelConnectionForm language={language} mutable={mutable} allowCustom={false} initialDraft={draft} existingNames={management.sources.map(s => s.display_name)} expectedRevisions={management.revisions} backend={backend}
    onBack={onBack} onCancel={() => { if (!uncertainRef.current) onBack(); }}
    onSaveAccepted={operation => { accepted.current = operation; onOperation(operation); }}
    onSaveResult={result => { if (result.disposition === 'saved') { setSaved(result); void confirm(result); } }}
    onSaveUncertain={() => { uncertainRef.current = true; setUncertain(true); }} restoreFocus={() => {}} />
  </section>;
}
