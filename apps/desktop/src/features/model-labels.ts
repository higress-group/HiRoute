export type ModelLabelIndex = ReadonlyMap<string, { name: string; sourceName: string | null; sourceState: 'single' | 'multiple' | 'unconfirmed'; disambiguator: string | null }>;
export type LabelSource = { source_id: string; display_name: string; binding_ids: readonly string[] };
type Candidate = { model_configuration_id: string; binding_id: string; display_name: string };

/** Current names and connections are presentation only; IDs remain exact lookup keys. */
export function buildModelLabelIndex(candidates: readonly Candidate[], sources?: readonly LabelSource[]): ModelLabelIndex {
  const configurations = new Map<string, { names: Set<string>; bindings: Set<string> }>();
  const bindingSources = new Map<string, LabelSource[]>();
  for (const source of sources ?? []) for (const binding of source.binding_ids) {
    const owners = bindingSources.get(binding) ?? [];
    owners.push(source);
    bindingSources.set(binding, owners);
  }
  for (const candidate of candidates) {
    const entry = configurations.get(candidate.model_configuration_id) ?? { names: new Set<string>(), bindings: new Set<string>() };
    entry.names.add(candidate.display_name);
    entry.bindings.add(candidate.binding_id);
    configurations.set(candidate.model_configuration_id, entry);
  }
  const result = new Map<string, { name: string; sourceName: string | null; sourceState: 'single' | 'multiple' | 'unconfirmed'; disambiguator: string | null }>();
  for (const [id, entry] of configurations) {
    if (!id || entry.names.size !== 1) continue;
    const name = [...entry.names][0];
    if (!name.trim()) continue;
    const owners = new Map<string, string>();
    let complete = Boolean(sources && entry.bindings.size);
    for (const binding of entry.bindings) {
      const matching = bindingSources.get(binding) ?? [];
      const ids = new Set(matching.map(source => source.source_id));
      const names = new Set(matching.map(source => source.display_name));
      if (!binding || ids.size !== 1 || names.size !== 1 || !matching[0]?.source_id || !matching[0].display_name.trim()) { complete = false; continue; }
      const [sourceId, sourceName] = [matching[0].source_id, matching[0].display_name];
      if (owners.has(sourceId) && owners.get(sourceId) !== sourceName) complete = false;
      owners.set(sourceId, sourceName);
    }
    const sourceState = !complete || !owners.size ? 'unconfirmed' : owners.size === 1 ? 'single' : 'multiple';
    result.set(id, { name, sourceState, sourceName: sourceState === 'single' ? [...owners.values()][0] : null, disambiguator: null });
  }
  const groups = new Map<string, string[]>();
  for (const [id, value] of result) groups.set(value.name, [...(groups.get(value.name) ?? []), id]);
  for (const ids of groups.values()) {
    if (ids.length < 2) continue;
    const qualifier = (id: string) => { const value = result.get(id)!; return value.sourceState === 'single' ? `single:${value.sourceName}` : value.sourceState; };
    for (const id of ids) {
      const collisions = ids.filter(other => qualifier(other) === qualifier(id));
      if (collisions.length < 2) continue;
      let length = Math.min(8, id.length);
      while (length < id.length && collisions.some(other => other !== id && other.startsWith(id.slice(0, length)))) length += 1;
      result.get(id)!.disambiguator = id.slice(0, length);
    }
  }
  return result;
}

export function modelLabel(configurationId: string | null | undefined, nativeModel: string | null | undefined, labels: ModelLabelIndex, language: 'zh' | 'en'): { text: string; detail: string | null; basis: 'configuration' | 'native' | 'unavailable' } {
  const value = configurationId == null ? undefined : labels.get(configurationId);
  if (value) {
    const duplicate = [...labels.values()].filter(item => item.name === value.name).length > 1;
    const source = !duplicate ? '' : value.sourceState === 'single' ? value.sourceName! : value.sourceState === 'multiple' ? (language === 'zh' ? '多个连接' : 'Multiple connections') : (language === 'zh' ? '来源未确认' : 'Source unconfirmed');
    const text = [value.name, source, value.disambiguator].filter(Boolean).join(' · ');
    const detail = [language === 'zh' ? '当前配置名称' : 'Current configuration name', configurationId, nativeModel, source ? `${language === 'zh' ? '当前连接' : 'Current connection'}: ${source}` : ''].filter(Boolean).join(' · ');
    return { text, detail, basis: 'configuration' };
  }
  const detail = configurationId ? `${language === 'zh' ? '配置 ID' : 'Configuration ID'}: ${configurationId}` : null;
  return nativeModel ? { text: nativeModel, detail, basis: 'native' } : { text: language === 'zh' ? '模型未确认' : 'Model unconfirmed', detail, basis: 'unavailable' };
}
