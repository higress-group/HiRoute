import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from './host-mock';
import { ModelManagementPage } from '../../../src/product/ModelManagementPage';
import { PresentationRoot } from '../../../src/ui';
import type { ManagementSnapshot } from '../../../src/features/models/types';
import type { SubscriptionCandidate, SubscriptionCheckResult } from '../../../src/features/subscriptions/types';
import { idleModelFeedStatus, readyManagement } from './product-fixtures';
import '../../../src/occami/styles.css';

const pending: SubscriptionCandidate = {
  candidate: { candidate_ref: 'candidate/cpa/codex/component', candidate_revision: 1 },
  correlation: { candidate_ref: 'candidate/cpa/codex/component', edit_revision: 1, check_id: 'check/component', input_digest: 'sha256:component' },
  producer: 'cpa', provenance: 'connector_owned', display_name: 'Codex 组件测试订阅',
  models: [], input_state: 'not_required', fact_state: 'pending_approval',
};
const approval = { operation_id: 'operation/component-check', state: 'succeeded', sequence: 2, cancellable: false };
const checked: SubscriptionCandidate = {
  ...pending,
  candidate: { ...pending.candidate, candidate_revision: 2 },
  fact_state: 'complete',
  validation: { approval_operation: approval, validation_ref: 'validation/component', validation_revision: '2' },
  models: [
    { model_ref: 'model/matched', upstream_model_id: 'matched', display_name: 'Matched model', membership: 'catalog', selectable: true },
    { model_ref: 'model/inventory-only', upstream_model_id: 'inventory-only', display_name: 'Inventory only', membership: 'observed', selectable: false, reason: 'inventory_only' },
    { model_ref: 'model/image-only', upstream_model_id: 'image-only', display_name: 'Image inventory', membership: 'observed', selectable: false, reason: 'inventory_only' },
  ],
};
const verified: SubscriptionCheckResult = { candidate: pending.candidate, approval_operation: approval, status: 'verified', checked_candidate: checked, validation: checked.validation };
const checking: SubscriptionCheckResult = { candidate: pending.candidate, approval_operation: { ...approval, state: 'running', cancellable: true }, status: 'checking' };
const baseManagement: ManagementSnapshot = structuredClone(readyManagement);
baseManagement.sources = [baseManagement.sources[0]];
baseManagement.sources[0].connection_identity = { access_kind: 'unknown', connection_option_id: null, product_label: null };
baseManagement.sources[0].actions = ['reauthorize'];
baseManagement.sources[0].models = [baseManagement.sources[0].models[0]];
baseManagement.sources[0].models[0].presentation = { billing_class: 'unknown', availability: 'unknown', reason_code: 'facts_unavailable', evaluated_at_ms: 1, price_contexts: [] };
baseManagement.runtime_state = 'partial';
const discovery = { discovery_ref: `discovery/${'a'.repeat(64)}`, discovery_revision: '1' };
const discovered = { connection_ref: 'connection-point/component', context_ref: 'native-scope/component', agent_id: 'agent_claude_default', display_name: '智谱 Coding Plan', configuration_state: 'configured', connection_option_id: 'zhipu.coding-plan.cn.v1', protocol: 'messages', declared_model_ids: ['GLM-5.3'], default_model_id: 'GLM-5.3', discovery };
const prepared = { ...checked, candidate: { candidate_ref: 'candidate/discovered/component', candidate_revision: 1 }, producer: 'native', provenance: 'registered', validation: undefined, display_name: 'Discovered component', models: [checked.models[0]] };
const saveOperation = { operation_id: 'operation/component-save', state: 'succeeded', sequence: 4, cancellable: false };

type Handler = (payload: Record<string, unknown>) => unknown;
const control = {
  handlers: {} as Record<string, Handler>,
  commands: [] as { command: string; payload: Record<string, unknown> }[],
  recoveryRefreshes: 0,
  changes: 0,
  management: structuredClone(baseManagement),
  subscriptions: [structuredClone(pending)],
  result: null as SubscriptionCheckResult | null,
  pending, checked, verified, checking, discovered, prepared, saveOperation,
  reset: () => {},
  refresh: () => {},
  setActive: (_active: boolean) => {},
};
Object.assign(window, { subscriptionRepair: control });

mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  control.commands.push({ command, payload: args });
  if (control.handlers[command]) return control.handlers[command](args);
  switch (command) {
    case 'compute_management_snapshot': return structuredClone(control.management);
    case 'compute_subscriptions': return { discovery_state: 'complete', candidates: structuredClone(control.subscriptions) };
    case 'compute_scan': return { connections: [discovered], diagnostics: [] };
    case 'recover_subscription_check': return control.result;
    case 'check_subscription':
      control.subscriptions = [structuredClone(checked)];
      control.result = structuredClone(verified);
      return control.result;
    case 'get_subscription_check_result': return control.result ?? verified;
    case 'close_subscription_check': return undefined;
    case 'prepare_discovered_model_connection': return prepared;
    case 'preview_compute_save': return { spec: { desired_state: args.change }, accept_digest: 'sha256:component', expected_revisions: control.management.revisions };
    case 'apply_compute_save': return { operation: saveOperation };
    case 'model_feed_status': return structuredClone(idleModelFeedStatus);
    case 'get_compute_save_result': return { disposition: 'saved', source_id: control.management.sources[0]?.source_id };
    default: throw new Error(`Unexpected component command: ${command}`);
  }
});

function Harness() {
  const [generation, setGeneration] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const [active, setActive] = useState(true);
  control.refresh = () => setRefresh(value => value + 1);
  control.setActive = setActive;
  control.reset = () => {
    control.handlers = {};
    control.commands = [];
    control.recoveryRefreshes = 0;
    control.changes = 0;
    control.management = structuredClone(baseManagement);
    control.subscriptions = [structuredClone(pending)];
    control.result = null;
    setActive(true);
    setRefresh(0);
    setGeneration(value => value + 1);
  };
  return <PresentationRoot language="zh" theme="dark" textScale={1}>
    <div className="app-window"><main className="main" style={{ marginLeft: 0 }}>
      <div role="note">组件测试：所有 IPC 为 mock；不连接 Tauri、daemon 或真实凭据。</div>
      <div hidden={!active}><ModelManagementPage key={generation} language="zh" active={active} trustedAuthority refreshVersion={refresh} plans={[]} agents={[]} onOperation={() => {}} onRecoveryRefresh={() => { control.recoveryRefreshes += 1; }} onChanged={() => { control.changes += 1; }} /></div>
    </main></div>
  </PresentationRoot>;
}

createRoot(document.getElementById('root')!).render(<Harness />);
