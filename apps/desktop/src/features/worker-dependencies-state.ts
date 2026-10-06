export type WorkerHarness = 'codex_cli' | 'claude_code' | 'qoder_cli' | 'pi' | 'deepseek_harness';
export type WorkerDependencyComponent = 'cli' | 'adapter' | 'node';
export type WorkerDependencyCandidateState = 'found' | 'missing' | 'invalid' | 'unavailable';

export type WorkerDependencyCandidate = {
  harness: WorkerHarness;
  component: WorkerDependencyComponent;
  path: string;
  source: 'selected' | 'path' | 'common' | 'npm_global' | 'npx_cache' | 'manual';
  state: WorkerDependencyCandidateState;
  reason_code?: string | null;
};

export type WorkerDependencySelection = {
  harness: WorkerHarness;
  adapter_path?: string | null;
  cli_path: string;
  node_path?: string | null;
};

export type WorkerDependenciesView = {
  schema: string;
  selection_revisions: { harness: WorkerHarness; revision: number }[];
  candidates: WorkerDependencyCandidate[];
  selected: WorkerDependencySelection[];
  install_hints: {
    harness: WorkerHarness;
    component: WorkerDependencyComponent;
    platform: string;
    command?: string | null;
    reason_code: string;
  }[];
};

export type WorkerDependencySelectionRequest = WorkerDependencySelection & {
  expected_selection_revision: number;
};

export function workerDependencyComponents(harness: WorkerHarness): WorkerDependencyComponent[] {
  return (harness === 'qoder_cli' || harness === 'deepseek_harness') ? ['cli'] : harness === 'pi' ? ['cli', 'node'] : ['cli', 'adapter', 'node'];
}

export function workerDependencySelectionComplete(selection: WorkerDependencySelection | null): boolean {
  if (!selection?.cli_path) return false;
  return (selection.harness === 'qoder_cli' || selection.harness === 'deepseek_harness')
    ? selection.adapter_path == null && selection.node_path == null
    : selection.harness === 'pi' ? selection.adapter_path == null && Boolean(selection.node_path) : Boolean(selection.adapter_path);
}

export type WorkerMachineEnvelope<T> = {
  status: 'succeeded' | 'accepted' | 'usage_error' | 'conflict' | 'denied' | 'not_found' | 'unavailable' | 'action_required' | 'needs_attention' | 'internal_error';
  data?: T | null;
  operation?: { operation_id: string; state: string; sequence: number; cancellable: boolean } | null;
  next_actions: { command_id: string; input: Record<string, unknown>; reason_code: string }[];
  error?: { code: string; message_key: string } | null;
};

export function workerEnvelopeData<T>(envelope: WorkerMachineEnvelope<T>): T {
  if (envelope.error || !['succeeded', 'accepted'].includes(envelope.status) || envelope.data == null) {
    const failure = new Error(envelope.error?.message_key ?? 'WORKER_RESPONSE_DATA_MISSING');
    Object.assign(failure, { code: envelope.error?.code ?? 'RESPONSE_DATA_MISSING', envelope });
    throw failure;
  }
  return envelope.data;
}

export function selectedWorkerDependencies(
  view: WorkerDependenciesView,
  harness: WorkerHarness,
): WorkerDependencySelection | null {
  return view.selected.find(selection => selection.harness === harness) ?? null;
}

export function workerDependencyRevision(view: WorkerDependenciesView, harness: WorkerHarness): number {
  return view.selection_revisions.find(revision => revision.harness === harness)?.revision ?? 0;
}

export function workerDependencySelectionMatches(
  view: WorkerDependenciesView,
  request: WorkerDependencySelectionRequest | null,
): boolean {
  if (!request) return false;
  const selected = selectedWorkerDependencies(view, request.harness);
  return Boolean(selected
    && selected.cli_path === request.cli_path
    && (selected.adapter_path ?? null) === (request.adapter_path ?? null)
    && (selected.node_path ?? null) === (request.node_path ?? null));
}

export function recommendedWorkerDependencies(
  view: WorkerDependenciesView,
  harness: WorkerHarness,
): WorkerDependencySelectionRequest {
  const selected = selectedWorkerDependencies(view, harness);
  if (selected) {
    return {
      ...selected,
      ...((harness === 'qoder_cli' || harness === 'deepseek_harness') ? {} : { node_path: selected.node_path ?? null }),
      expected_selection_revision: workerDependencyRevision(view, harness),
    };
  }
  const found = (component: WorkerDependencyComponent) => view.candidates.find(candidate =>
    candidate.harness === harness && candidate.component === component && candidate.state === 'found');
  return {
    harness,
    cli_path: found('cli')?.path ?? '',
    ...((harness === 'qoder_cli' || harness === 'deepseek_harness') ? {} : {
      ...(harness === 'pi' ? {} : { adapter_path: found('adapter')?.path ?? '' }),
      node_path: found('node')?.path ?? null,
    }),
    expected_selection_revision: workerDependencyRevision(view, harness),
  };
}

export function workerDependencySelectionState(
  view: WorkerDependenciesView,
  harness: WorkerHarness,
): 'configured' | 'found' | 'incomplete' {
  const request = recommendedWorkerDependencies(view, harness);
  const selected = selectedWorkerDependencies(view, harness);
  const fact = (component: WorkerDependencyComponent, path: string | null | undefined) => path
    ? view.candidates.find(candidate => candidate.harness === harness
        && candidate.component === component
        && candidate.path === path)?.state
    : undefined;
  const nodeRequired = harness === 'pi' || harness !== 'qoder_cli' && harness !== 'deepseek_harness' && (Boolean(request.node_path)
    || (!selected && view.install_hints.some(hint => hint.harness === harness
      && hint.component === 'node'
      && hint.reason_code === 'worker.dependencies.install_required')));
  const complete = workerDependencySelectionComplete(request)
    && fact('cli', request.cli_path) === 'found'
    && (harness === 'qoder_cli' || harness === 'deepseek_harness' || harness === 'pi' || fact('adapter', request.adapter_path) === 'found')
    && (!nodeRequired || fact('node', request.node_path) === 'found');
  if (selected && complete) return 'configured';
  if (complete) return 'found';
  return 'incomplete';
}
