import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AgentSnapshot } from '../agents';
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
  projectHomeAgents,
  projectHomeCompute,
  projectHomePlans,
  projectHomeService,
  projectHomeValue,
  type DesktopSnapshot,
  type ObservationSessionPage,
  type ObservationValueSummary,
} from './home-projections';
import { safeDiagnosticCode } from '../error-code';

const HOME_TARGET = 'desktop/local-home';
const loading = <T,>(): DomainReadSlot<T> => ({
  requestId: null,
  targetKey: HOME_TARGET,
  read: { status: 'loading' },
});

function errorCode(error: unknown): string {
  return safeDiagnosticCode(error, 'READ_UNAVAILABLE');
}

function observationRequest(view: string, query: object) {
  return {
    request: {
      schema: 'hiroute.observation.query/v2',
      intent: { view, query },
    },
  };
}

export function useDesktopHome() {
  const [startup, setStartup] = useState<{ state: 'starting' | 'ready' | 'failed'; code?: string; recovery_available: boolean; backup_directory?: string; upgrade_phase?: 'source_check' | 'backup' | 'conversion' | 'validation' | 'service_recovery' } | null>(null);
  const [service, setService] = useState<DomainReadSlot<HomeService>>(loading);
  const [compute, setCompute] = useState<DomainReadSlot<HomeCompute>>(loading);
  const [plans, setPlans] = useState<DomainReadSlot<HomePlans>>(loading);
  const [agents, setAgents] = useState<DomainReadSlot<HomeAgents>>(loading);
  const [activity, setActivity] = useState<DomainReadSlot<HomeActivity>>(loading);
  const [value, setValue] = useState<DomainReadSlot<HomeValue>>(loading);
  const [desktopSnapshot, setDesktopSnapshot] = useState<DesktopSnapshot | null>(null);
  const [agentSnapshot, setAgentSnapshot] = useState<AgentSnapshot | null>(null);
  const sequence = useRef(0);
  const activeDesktopRequest = useRef<string | null>(null);

  const requestId = useCallback((domain: keyof HomeReads) =>
    `home/${domain}/${++sequence.current}`, []);

  const refreshDesktop = useCallback(async () => {
    const serviceRequest = requestId('service');
    const plansRequest = requestId('plans');
    activeDesktopRequest.current = serviceRequest;
    setService(current => beginDomainRead(current, serviceRequest, HOME_TARGET));
    setPlans(current => beginDomainRead(current, plansRequest, HOME_TARGET));
    try {
      const snapshot = await invoke<DesktopSnapshot>('desktop_snapshot');
      setStartup({ state: 'ready', recovery_available: false });
      if (activeDesktopRequest.current !== serviceRequest) return;
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
      try {
        setStartup(await invoke('startup_status'));
      } catch {
        setStartup({ state: 'failed', code, recovery_available: false });
      }
    }
  }, [requestId]);

  const refreshCompute = useCallback(async () => {
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
    setAgents(current => beginDomainRead(current, request, HOME_TARGET));
    try {
      const snapshot = await invoke<AgentSnapshot>('agent_snapshot');
      setAgentSnapshot(snapshot);
      setAgents(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeAgents(snapshot),
      ));
    } catch (error) {
      setAgents(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  const refreshActivity = useCallback(async () => {
    const request = requestId('activity');
    setActivity(current => beginDomainRead(current, request, HOME_TARGET));
    const to = Date.now();
    try {
      const page = await invoke<ObservationSessionPage>(
        'observation_read',
        observationRequest('sessions', {
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
        }),
      );
      setActivity(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeActivity(page),
      ));
    } catch (error) {
      setActivity(current => failDomainRead(
        current,
        request,
        HOME_TARGET,
        errorCode(error),
      ));
    }
  }, [requestId]);

  const refreshValue = useCallback(async () => {
    const request = requestId('value');
    setValue(current => beginDomainRead(current, request, HOME_TARGET));
    try {
      const summary = await invoke<ObservationValueSummary>(
        'observation_read',
        observationRequest('home_value', {
          period: 'seven_days',
          session_id: null,
          currency: null,
        }),
      );
      setValue(current => resolveDomainRead(
        current,
        request,
        HOME_TARGET,
        projectHomeValue(summary),
      ));
    } catch (error) {
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

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const report = await invoke<NonNullable<typeof startup>>('startup_status');
        if (cancelled) return;
        setStartup(current => current?.state === 'ready' ? current : report);
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
    agentSnapshot,
    refreshAll,
    refreshDesktop,
    refreshCompute,
    refreshAgents,
    refreshActivity,
    refreshValue,
    refreshDomain,
  };
}
