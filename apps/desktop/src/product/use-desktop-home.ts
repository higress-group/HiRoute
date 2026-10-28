import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '../host/index.ts';
import {
  beginDomainRead,
  failDomainRead,
  resolveDomainRead,
  type DomainReadSlot,
} from '../features/home/state';
import type {
  HomeActivity,
  HomeAgents,
  HomeCompute,
  HomePlans,
  HomeReads,
  HomeService,
  HomeValue,
} from '../features/home/types';
import type { ManagementSnapshot } from '../features/models/types';
import {
  projectHomeActivity,
  projectHomeAgentSummary,
  projectHomeCompute,
  projectHomePlans,
  projectHomeService,
  projectHomeValue,
  type DesktopSnapshot,
  type AgentHomeSummary,
  type ObservationSessionPage,
  type ObservationValueSummary,
} from './home-projections';
import { safeDiagnosticCode } from '../error-code';
import { readHomeObservation } from './home-observation';

const HOME_TARGET = 'desktop/local-home';
const loading = <T,>(): DomainReadSlot<T> => ({
  requestId: null,
  targetKey: HOME_TARGET,
  read: { status: 'loading' },
});

function errorCode(error: unknown): string {
  return safeDiagnosticCode(error, 'READ_UNAVAILABLE');
}

export function useDesktopHome() {
  const [startup, setStartup] = useState<{ state: 'starting' | 'ready' | 'failed'; code?: string; storage_recovery?: unknown; recovery_available: boolean; backup_directory?: string; upgrade_phase?: 'source_check' | 'backup' | 'conversion' | 'validation' | 'service_recovery' } | null>(null);
  const [service, setService] = useState<DomainReadSlot<HomeService>>(loading);
  const [compute, setCompute] = useState<DomainReadSlot<HomeCompute>>(loading);
  const [plans, setPlans] = useState<DomainReadSlot<HomePlans>>(loading);
  const [agents, setAgents] = useState<DomainReadSlot<HomeAgents>>(loading);
  const [activity, setActivity] = useState<DomainReadSlot<HomeActivity>>(loading);
  const [value, setValue] = useState<DomainReadSlot<HomeValue>>(loading);
  const [desktopSnapshot, setDesktopSnapshot] = useState<DesktopSnapshot | null>(null);
  const [agentDirectory, setAgentDirectory] = useState<AgentHomeSummary['agents'] | null>(null);
  const sequence = useRef(0);
  const activeDesktopRequest = useRef<string | null>(null);
  const activeAgentsRequest = useRef<string | null>(null);
  const invalidatedCatalog = useRef({ agents: false, compute: false });
  const [catalogInvalidation, setCatalogInvalidation] = useState(0);
  const startupSequence = useRef(0);
  const activityRead = useRef<AbortController | null>(null);
  const valueRead = useRef<AbortController | null>(null);

  const requestId = useCallback((domain: keyof HomeReads) =>
    `home/${domain}/${++sequence.current}`, []);

  const invalidateService = useCallback((connectionLost = false) => {
    activeDesktopRequest.current = null;
    setService({
      requestId: null,
      targetKey: HOME_TARGET,
      read: connectionLost
        ? { status: 'ready', data: { daemon: 'unavailable', gateway: 'unknown', recoveryReady: false, nativeConnectionLost: true } }
        : { status: 'loading' },
    });
  }, []);

  const refreshDesktop = useCallback(async () => {
    const serviceRequest = requestId('service');
    const plansRequest = requestId('plans');
    activeDesktopRequest.current = serviceRequest;
    setService(current => beginDomainRead(
      current.read.status === 'error' ? { ...current, read: { status: 'loading' } } : current,
      serviceRequest,
      HOME_TARGET,
    ));
    setPlans(current => beginDomainRead(current, plansRequest, HOME_TARGET));
    try {
      const snapshot = await invoke<DesktopSnapshot>('desktop_snapshot');
      if (activeDesktopRequest.current !== serviceRequest) return;
      startupSequence.current++;
      setStartup({ state: 'ready', recovery_available: false });
      setDesktopSnapshot(snapshot);
      setService(current => resolveDomainRead(
        current,
        serviceRequest,
        HOME_TARGET,
        projectHomeService(snapshot),
        snapshot.service.revisions ? String(snapshot.service.revisions.target) : undefined,
      ));
      if (snapshot.catalog_error) {
        setPlans(current => failDomainRead(
          current,
          plansRequest,
          HOME_TARGET,
          errorCode(snapshot.catalog_error),
        ));
      } else {
        setPlans(current => resolveDomainRead(
          current,
          plansRequest,
          HOME_TARGET,
          projectHomePlans(snapshot),
          snapshot.service.revisions ? String(snapshot.service.revisions.target) : undefined,
        ));
      }
    } catch (error) {
      if (activeDesktopRequest.current !== serviceRequest) return;
      const code = errorCode(error);
      setService(current => failDomainRead(current, serviceRequest, HOME_TARGET, code));
      setPlans(current => failDomainRead(current, plansRequest, HOME_TARGET, code));
      const startupRequest = ++startupSequence.current;
      const report = await invoke<NonNullable<typeof startup>>('startup_status').catch(() => ({
        state: 'failed' as const, code, recovery_available: false,
      }));
      if (activeDesktopRequest.current === serviceRequest && startupSequence.current === startupRequest) {
        setStartup(report);
      }
    }
  }, [requestId]);

  const refreshCompute = useCallback(async () => {
    invalidatedCatalog.current.compute = false;
    const request = requestId('compute');
    setCompute(current => beginDomainRead(current, request, HOME_TARGET));
    try {
      const snapshot = await invoke<ManagementSnapshot>('compute_management_snapshot');
      setCompute(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeCompute(snapshot),
        String(snapshot.revisions.target),
      ));
    } catch (error) {
      setCompute(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  const refreshAgents = useCallback(async () => {
    const request = requestId('agents');
    invalidatedCatalog.current.agents = false;
    activeAgentsRequest.current = request;
    setAgents(current => beginDomainRead(current, request, HOME_TARGET));
    try {
      const snapshot = await invoke<AgentHomeSummary>('agent_home_summary');
      if (activeAgentsRequest.current !== request) return;
      setAgentDirectory(snapshot.agents);
      setAgents(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeAgentSummary(snapshot),
      ));
    } catch (error) {
      if (activeAgentsRequest.current === request) setAgentDirectory(null);
      setAgents(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  // A mutation invalidates background summaries; it does not synchronously
  // scan every Agent or subscription before the active editor can confirm.
  const invalidateCatalog = useCallback(() => {
    invalidatedCatalog.current = { agents: true, compute: true };
    activeAgentsRequest.current = requestId('agents');
    setAgentDirectory(null);
    setAgents(current => beginDomainRead(current, activeAgentsRequest.current!, HOME_TARGET));
    const computeRequest = requestId('compute');
    setCompute(current => beginDomainRead(current, computeRequest, HOME_TARGET));
    setCatalogInvalidation(value => value + 1);
  }, [requestId]);
  const refreshInvalidatedCatalog = useCallback(async () => {
    await Promise.allSettled([
      ...(invalidatedCatalog.current.agents ? [refreshAgents()] : []),
      ...(invalidatedCatalog.current.compute ? [refreshCompute()] : []),
    ]);
  }, [refreshAgents, refreshCompute]);

  const refreshActivity = useCallback(async () => {
    activityRead.current?.abort();
    const controller = new AbortController();
    activityRead.current = controller;
    const request = requestId('activity');
    setActivity(current => beginDomainRead(current, request, HOME_TARGET));
    const to = Date.now();
    try {
      const page = await readHomeObservation<ObservationSessionPage>(
        'sessions', {
          from_ms: to - 7 * 86_400_000,
          to_ms: to,
          session_id: null,
          limit: 6,
          cursor: null,
          agent_id: null,
          plan_id: null,
          native_model: null,
          outcome: null,
          only_model_switch: false,
        }, controller.signal,
      );
      setActivity(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeActivity(page),
      ));
    } catch (error) {
      if (controller.signal.aborted) return;
      setActivity(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  const refreshValue = useCallback(async () => {
    valueRead.current?.abort();
    const controller = new AbortController();
    valueRead.current = controller;
    const request = requestId('value');
    setValue(current => beginDomainRead(current, request, HOME_TARGET));
    try {
      const summary = await readHomeObservation<ObservationValueSummary>(
        'home_value', {
          period: 'seven_days',
          session_id: null,
          currency: null,
        }, controller.signal,
      );
      setValue(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeValue(summary),
      ));
    } catch (error) {
      if (controller.signal.aborted) return;
      setValue(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  const refreshAll = useCallback(async () => {
    await Promise.allSettled([
      refreshDesktop(),
      refreshCompute(),
      refreshAgents(),
      refreshActivity(),
      refreshValue(),
    ]);
  }, [refreshActivity, refreshAgents, refreshCompute, refreshDesktop, refreshValue]);

  const refreshDomain = useCallback(async (domain: keyof HomeReads) => {
    if (domain === 'service' || domain === 'plans') return refreshDesktop();
    if (domain === 'compute') return refreshCompute();
    if (domain === 'agents') return refreshAgents();
    if (domain === 'activity') return refreshActivity();
    return refreshValue();
  }, [refreshActivity, refreshAgents, refreshCompute, refreshDesktop, refreshValue]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  useEffect(() => () => {
    activityRead.current?.abort();
    valueRead.current?.abort();
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const startupRequest = ++startupSequence.current;
      try {
        const report = await invoke<NonNullable<typeof startup>>('startup_status');
        if (cancelled) return;
        if (startupSequence.current === startupRequest) {
          setStartup(current => current?.state === 'ready' ? current : report);
        }
        if (report.state === 'starting') timer = setTimeout(() => void poll(), 500);
      } catch {
        if (!cancelled) timer = setTimeout(() => void poll(), 1000);
      }
    };
    void poll();
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, []);

  return {
    startup,
    reads: {
      service: service.read,
      compute: compute.read,
      plans: plans.read,
      agents: agents.read,
      activity: activity.read,
      value: value.read,
    } satisfies HomeReads,
    desktopSnapshot,
    agentDirectory,
    refreshAll,
    refreshDesktop,
    invalidateService,
    refreshCompute,
    refreshAgents,
    refreshActivity,
    refreshValue,
    refreshDomain,
    invalidateCatalog,
    refreshInvalidatedCatalog,
    catalogInvalidation,
  };
}
