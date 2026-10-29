import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../host/index.ts';
import { preferredTrialAgent, selectedTrialSnapshot, type TrialAgentChoices, type TrialAgentModel, type TrialAgentSnapshot } from './agent-read';

/** No persistent cache: choices last for this flow; each detail read validates current facts. */
export function useTrialAgents(agentId: string, initialChoices?: TrialAgentChoices | null) {
  const [snapshot, setSnapshot] = useState<TrialAgentSnapshot | null>(null);
  const [loading, setLoading] = useState(true), [error, setError] = useState(false);
  const live = useRef(false), generation = useRef(0), selected = useRef(agentId);
  const choices = useRef<TrialAgentChoices | null>(initialChoices ?? null), current = useRef<TrialAgentSnapshot | null>(null);
  const requestFor = useRef<string | null>(null);
  selected.current = agentId;

  async function refresh(discover = false): Promise<TrialAgentSnapshot | undefined> {
    const ticket = ++generation.current;
    const requested = selected.current;
    requestFor.current = requested;
    const valid = () => live.current && ticket === generation.current && selected.current === requested;
    setLoading(true); setError(false);
    try {
      const list = discover || !choices.current ? await invoke<TrialAgentChoices>('trial_agent_choices') : choices.current;
      if (!valid()) return;
      choices.current = list;
      setSnapshot({ ...list, selected: null, plans: current.current?.plans ?? { plans: [] } });
      const choice = preferredTrialAgent(list.choices, requested);
      if (!choice && requested) throw Error('AGENT_NOT_DISCOVERED');
      const next: TrialAgentSnapshot = choice
        ? selectedTrialSnapshot(list, choice, await invoke<TrialAgentModel>('trial_agent_model', { contextId: choice.context_id }))
        : { ...list, selected: null, plans: { plans: [] } };
      if (!valid()) return;
      current.current = next; setSnapshot(next);
      return next;
    } catch {
      if (valid()) { current.current = null; setError(true); }
    } finally {
      if (valid()) setLoading(false);
    }
  }
  useEffect(() => {
    // A later step in this same flow may retain discovery hints, never the
    // selected settings. Every mount still reads their current strict proof.
    live.current = true; void refresh(!choices.current);
    return () => { live.current = false; ++generation.current; };
  }, []);
  useEffect(() => {
    if (requestFor.current === agentId) return;
    // Adopting the default chosen by the completed first read needs no duplicate request.
    if (!loading && current.current?.selected?.agent_id === agentId) { requestFor.current = agentId; return; }
    void refresh();
  }, [agentId]);

  // Even the render before the selection effect runs cannot expose the previous Agent as ready.
  const matches = !agentId || snapshot?.selected?.agent_id === agentId;
  const ready = matches && !loading && !error;
  return { snapshot, loading: loading || (!matches && !error), error, ready, refresh };
}
