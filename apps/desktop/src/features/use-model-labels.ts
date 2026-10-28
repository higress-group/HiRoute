import { useEffect, useRef, useState } from 'react';
import { invoke } from '../host';
import { buildModelLabelIndex, type LabelSource, type ModelLabelIndex } from './model-labels';

type Candidate = { model_configuration_id: string; binding_id: string; display_name: string };
type Snapshot = { sources: { source_id: string; display_name: string; models: { binding_id: string }[] }[] };
const empty: ModelLabelIndex = new Map();

/** One narrow read per active page lifecycle; source details are loaded only for name collisions. */
export function useModelLabels(active: boolean, refreshVersion: number): ModelLabelIndex {
  const [result, setResult] = useState<{ version: number; labels: ModelLabelIndex }>({ version: refreshVersion, labels: empty });
  const pending = useRef<Promise<void>>(Promise.resolve());
  useEffect(() => {
    let current = true;
    const invalidate = () => { current = false; setResult({ version: refreshVersion, labels: empty }); };
    window.addEventListener('hiroute-native-connection-unavailable', invalidate);
    setResult({ version: refreshVersion, labels: empty });
    if (active) pending.current = pending.current.catch(() => {}).then(async () => {
      if (!current) return;
      const response = await invoke<{ candidates: Candidate[] }>('plan_editor_options', { input: { requirements: {}, native_selections: {}, suggest_free: false } });
      if (!current) return;
      const candidates = response.candidates.map(({ model_configuration_id, binding_id, display_name }) => ({ model_configuration_id, binding_id, display_name }));
      const initial = buildModelLabelIndex(candidates);
      setResult({ version: refreshVersion, labels: initial });
      const names = [...initial.values()].map(value => value.name);
      if (new Set(names).size === names.length) return;
      try {
        const snapshot = await invoke<Snapshot>('compute_management_snapshot');
        if (!current) return;
        const sources: LabelSource[] = snapshot.sources.map(({ source_id, display_name, models }) => ({ source_id, display_name, binding_ids: models.map(model => model.binding_id) }));
        setResult({ version: refreshVersion, labels: buildModelLabelIndex(candidates, sources) });
      } catch { /* Existing names and explicit source uncertainty remain usable. */ }
    }).catch(() => { /* The timeline remains independent of name lookup. */ });
    return () => { current = false; window.removeEventListener('hiroute-native-connection-unavailable', invalidate); };
  }, [active, refreshVersion]);
  return active && result.version === refreshVersion ? result.labels : empty;
}
