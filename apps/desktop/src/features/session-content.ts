export type ReadableEvent = { kind: 'text' | 'reasoning' | 'refusal'; text: string; sequence: number; index: number; phase: 'delta' | 'finished' } | { kind: 'tool'; sequence: number; name: string; logicalId: string; phase: 'started' | 'arguments' | 'ready'; arguments: string } | { kind: 'block'; block: string };

// These names come from the canonical Gateway content contract, not text patterns.
export function isPrivateContentKind(kind: string): boolean {
  return kind === 'provider_state' || kind === 'reasoning_delta' || kind === 'reasoning_finished';
}

export function isTechnicalContentKind(kind: string): boolean {
  return kind === 'message_name' || kind.startsWith('tool_');
}

export function coalesceResponseText(events: readonly ReadableEvent[]): { kind: 'text' | 'refusal'; index: number; text: string }[] {
  const parts = new Map<string, Extract<ReadableEvent, { kind: 'text' | 'reasoning' | 'refusal' }>[]>();
  for (const event of events) {
    if (event.kind !== 'text' && event.kind !== 'refusal') continue;
    const key = `${event.kind}\u0000${event.index}`;
    const group = parts.get(key) ?? [];
    group.push(event);
    parts.set(key, group);
  }
  return [...parts.values()].map(group => {
    group.sort((a, b) => a.sequence - b.sequence);
    const finished = group.filter(event => event.phase === 'finished').at(-1);
    return { kind: group[0].kind as 'text' | 'refusal', index: group[0].index, text: finished?.text ?? group.filter(event => event.phase === 'delta').map(event => event.text).join('') };
  }).sort((a, b) => a.index - b.index);
}

export function contentDisplayRuns(messages: readonly { role: string; technical?: boolean }[]): { start: number; end: number; technical: boolean }[] {
  const runs: { start: number; end: number; technical: boolean }[] = [];
  for (let index = 0; index < messages.length; index++) {
    const technical = messages[index].technical ?? ['system', 'developer', 'tool_definition'].includes(messages[index].role);
    const last = runs.at(-1);
    if (last?.technical === technical) last.end = index + 1;
    else runs.push({ start: index, end: index + 1, technical });
  }
  return runs;
}

export function readableEvent(raw: string, mediaType: string, direction: string): ReadableEvent | null {
  if (mediaType !== 'application/vnd.hiroute.model-stream-event+json;version=1' || direction !== 'response_delivered') return null;
  try {
    const value = JSON.parse(raw);
    if (value?.schema_version !== 'hiroute.model-stream-event/v1' || !Number.isSafeInteger(value.sequence) || value.sequence < 0) return null;
    const event = value.event;
    if (!event || typeof event !== 'object') return null;
    const kinds: Record<string, 'text' | 'reasoning' | 'refusal'> = { text_delta: 'text', text_finished: 'text', reasoning_delta: 'reasoning', reasoning_finished: 'reasoning', refusal_delta: 'refusal', refusal_finished: 'refusal' };
    if (kinds[event.kind] && typeof event.text === 'string') return { kind: kinds[event.kind], text: event.text, sequence: value.sequence, index: Number.isSafeInteger(event.index) && event.index >= 0 ? event.index : 0, phase: event.kind.endsWith('_finished') ? 'finished' : 'delta' };
    if (event.kind === 'content_block_started' && typeof event.block_kind === 'string') return { kind: 'block', block: event.block_kind };
    if (typeof event.logical_id !== 'string') return null;
    if (event.kind === 'tool_arguments_delta' && typeof event.delta === 'string') return { kind: 'tool', sequence: value.sequence, logicalId: event.logical_id, name: '', phase: 'arguments', arguments: event.delta };
    if ((event.kind === 'tool_call_started' || event.kind === 'tool_call_finished') && typeof event.name === 'string' && (event.namespace == null || typeof event.namespace === 'string')) return { kind: 'tool', sequence: value.sequence, logicalId: event.logical_id, name: [event.namespace, event.name].filter(Boolean).join('.'), phase: event.kind === 'tool_call_started' ? 'started' : 'ready', arguments: event.arguments == null ? '' : JSON.stringify(event.arguments, null, 2) };
  } catch { /* Partial chunks and unknown data remain available as original content. */ }
  return null;
}
export function excerpt(text: string, limit = 160): string {
  const chars = Array.from(text.replace(/\s+/gu, ' ').trim());
  return chars.slice(0, limit).join('') + (chars.length > limit ? '…' : '');
}

export function summarizeToolArguments(raw: string, limit = 220): string {
  const scalar = (value: unknown): string => {
    if (typeof value === 'string') return value;
    if (typeof value === 'number' || typeof value === 'boolean' || value === null) return String(value);
    if (Array.isArray(value) && value.every(item => ['string', 'number', 'boolean'].includes(typeof item))) return value.join(', ');
    try { return JSON.stringify(value); } catch { return ''; }
  };
  try {
    const value = JSON.parse(raw);
    if (!value || typeof value !== 'object' || Array.isArray(value)) return excerpt(scalar(value), limit);
    const entries = Object.entries(value).slice(0, 4);
    if (entries.length === 1 && /^(path|paths|file|files|query|command|cmd)$/i.test(entries[0][0])) return excerpt(scalar(entries[0][1]), limit);
    return excerpt(entries.map(([key, item]) => `${key}: ${scalar(item)}`).join(' · '), limit);
  } catch {
    return excerpt(raw, limit);
  }
}

export function groupToolEvents(events: ReadableEvent[]): Extract<ReadableEvent, { kind: 'tool' }>[] {
  const grouped = new Map<string, Extract<ReadableEvent, { kind: 'tool' }>>();
  const ordered = events.filter((e): e is Extract<ReadableEvent, { kind: 'tool' }> => e.kind === 'tool').sort((a, b) => a.sequence - b.sequence);
  for (const event of ordered) {
    const previous = grouped.get(event.logicalId);
    if (event.phase === 'arguments') grouped.set(event.logicalId, { ...event, name: previous?.name ?? '', arguments: `${previous?.arguments ?? ''}${event.arguments}` });
    else grouped.set(event.logicalId, { ...event, name: event.name || previous?.name || '', arguments: event.arguments || previous?.arguments || '' });
  }
  return [...grouped.values()];
}

// Search anchors use original UTF-8 byte offsets, never JavaScript character indexes.
export function excerptAround(text: string, byteOffset: number, limit = 160): string {
  const bytes = new TextEncoder().encode(text);
  const prefix = new TextDecoder().decode(bytes.slice(0, Math.min(bytes.length, byteOffset)));
  const chars = Array.from(text);
  const start = Math.max(0, Array.from(prefix).length - Math.floor(limit / 3));
  return (start ? '…' : '') + excerpt(chars.slice(start).join(''), limit);
}
