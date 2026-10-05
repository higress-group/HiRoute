/** Resolve historical execution names through the existing authorized request query.
 * Configuration IDs are opaque; a current plan or another stage is not evidence
 * of which model executed a retained stage. Never inspect ID prefixes to guess.
 */
type Stage = {
  segment_id: string;
  session_id: string;
  attribution: 'single' | 'mixed' | 'unknown';
  first_request_id?: string | null;
  last_request_id?: string | null;
  execution_evidence_available: boolean;
};
type RequestModel = { session_id: string; request_id: string; final_native_model: string | null };
type Lookup = (session: string, request: string) => Promise<readonly RequestModel[]>;

export async function qualityExecutionModels(stages: readonly Stage[], lookup: Lookup): Promise<Record<string, string>> {
  const requests = new Map<string, Promise<string | null>>();
  const read = (session: string, request: string) => {
    const key = JSON.stringify([session, request]);
    if (!requests.has(key)) requests.set(key, lookup(session, request)
      .then(rows => rows.find(row => row.session_id === session && row.request_id === request)?.final_native_model?.trim() || null)
      .catch(() => null));
    return requests.get(key)!;
  };
  const names: Record<string, string> = {};
  let next = 0;
  // The observation service admits four concurrent queries. Leave room for the
  // surrounding session's transcript and facts instead of firing a whole page.
  await Promise.all(Array.from({ length: Math.min(2, stages.length) }, async () => {
    while (next < stages.length) {
      const stage = stages[next++];
      if (stage.attribution !== 'single' || !stage.execution_evidence_available) continue;
      for (const request of new Set([stage.last_request_id, stage.first_request_id])) {
        if (!request) continue;
        const name = await read(stage.session_id, request);
        if (name) { names[stage.segment_id] = name; break; }
      }
    }
  }));
  return names;
}
