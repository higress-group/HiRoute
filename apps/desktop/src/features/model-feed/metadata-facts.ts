type Value = Record<string, unknown>;
export type MetadataReference = {kind: string; key: string};
function object(value: unknown): Value {
  return value !== null && typeof value === 'object' ? value as Value : {};
}
export function inferenceRule(value: Value, field: string): string | null {
  const provenance = object(object(value.field_provenance)[field]);
  return provenance.basis === 'inferred' && typeof provenance.rule_key === 'string' ? provenance.rule_key : null;
}
export function metadataReferences(value: Value): MetadataReference[] {
  const references: MetadataReference[] = [];
  if (Array.isArray(value.provenance_refs)) for (const raw of value.provenance_refs) {
    const ref = object(raw);
    const kind = ref.kind === 'evidence-source' ? 'evidence_source' : ref.kind === 'client-discovery-run' ? 'client_run' : null;
    if (kind && typeof ref.source_key === 'string') references.push({kind, key: ref.source_key});
  }
  for (const field of Object.keys(object(value.field_provenance))) {
    const key = inferenceRule(value, field);
    if (key) references.push({kind:'inference_rule',key});
  }
  return references.filter((ref,index) => references.findIndex(other => other.kind === ref.kind && other.key === ref.key) === index);
}
export function capabilityText(value: unknown, zh: boolean): string {
  if (value === true || value === 'supported') return zh ? '支持' : 'Supported';
  if (value === false || value === 'unsupported') return zh ? '不支持' : 'Not supported';
  const labels: Record<string, [string,string]> = {
    conditional:['有条件支持','Conditional'], conflict:['资料存在冲突','Conflicting information'],
    runtime_required:['需要实际确认','Requires qualification'], not_applicable:['不适用','Not applicable'],
  };
  return labels[String(value).replaceAll('-','_')]?.[zh ? 0 : 1] ?? (zh ? '未知' : 'Unknown');
}
export function tokenLimitText(value: unknown, zh: boolean): string {
  if (typeof value === 'number') return value.toLocaleString();
  const limit = object(value);
  if (limit.state === 'known' && typeof limit.value === 'number') return limit.value.toLocaleString();
  if (limit.state === 'entitlement_dependent' || limit.state === 'entitlement-dependent') return zh ? '取决于账户权益' : 'Depends on account entitlement';
  if (limit.state === 'shared-total-budget') return zh ? '输入与输出共用额度' : 'Shared input/output budget';
  if (limit.state === 'runtime-required') return zh ? '需要实际确认' : 'Requires qualification';
  if (limit.state === 'not_applicable' || limit.state === 'not-applicable') return zh ? '不适用' : 'Not applicable';
  if (limit.state === 'conflict') return zh ? '资料存在冲突' : 'Conflicting information';
  return zh ? '未知' : 'Unknown';
}
