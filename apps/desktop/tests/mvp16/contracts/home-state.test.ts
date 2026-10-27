import assert from 'node:assert/strict';
import test from 'node:test';
import { operationLabel } from '../../../src/features/home/copy.ts';
import * as homeState from '../../../src/features/home/state.ts';
import {
  acceptOperation,
  beginDomainRead,
  candidateFactPresentation,
  deriveHomePrimaryMode,
  failDomainRead,
  isExplicitlyFree,
  resolveDomainRead,
  subscriptionPresentation,
  visibleData,
  type DomainReadSlot,
} from '../../../src/features/home/state.ts';
import type { HomeReads } from '../../../src/features/home/types.ts';

const emptyReads = (): HomeReads => ({
  service: { status: 'ready', data: { daemon: 'running', gateway: 'empty', recoveryReady: true } },
  compute: { status: 'ready', data: { candidates: [], sources: [], saveAttempts: [], subscriptionChecks: [] } },
  plans: { status: 'ready', data: { plans: [], drafts: [] } },
  agents: { status: 'ready', data: { agents: [] } },
  activity: { status: 'ready', data: { sessions: [], tasks: [] } },
  value: { status: 'ready', data: { rangeLabel: 'today', coverage: 'unknown', pending: 0, provisional: false, money: [] } },
});

test('first-use requires successful empty domain reads', () => {
  assert.equal(deriveHomePrimaryMode(emptyReads()), 'first-use');
  const loading = emptyReads();
  loading.compute = { status: 'loading' };
  assert.equal(deriveHomePrimaryMode(loading), 'loading');
  const failed = emptyReads();
  failed.compute = { status: 'error', code: 'READ_FAILED' };
  assert.equal(deriveHomePrimaryMode(failed), 'loading');
});

test('user-configured sources remain visible without catalog intersection', () => {
  const reads = emptyReads();
  reads.compute = { status: 'ready', data: { candidates: [], sources: [{ sourceId: 'source/custom', displayName: 'Custom', bindingIds: ['binding/custom'], origin: 'user_configured', authentication: 'header', capabilityKnowledge: 'unknown', priceKnowledge: 'unknown' }], saveAttempts: [], subscriptionChecks: [] } };
  assert.equal(visibleData(reads.compute)?.sources[0]?.origin, 'user_configured');
  assert.equal(deriveHomePrimaryMode(reads), 'partial');
});

test('late query results cannot overwrite the current target', () => {
  let slot: DomainReadSlot<string[]> = { requestId: null, targetKey: null, read: { status: 'ready', data: ['old'] } };
  slot = beginDomainRead(slot, 'request-1', 'workspace-a');
  slot = beginDomainRead(slot, 'request-2', 'workspace-b');
  const late = resolveDomainRead(slot, 'request-1', 'workspace-a', ['late']);
  assert.equal(late, slot);
  const current = resolveDomainRead(slot, 'request-2', 'workspace-b', ['current']);
  assert.deepEqual(visibleData(current.read), ['current']);
});

test('one failed refresh keeps that domain previous data', () => {
  let slot: DomainReadSlot<string[]> = { requestId: null, targetKey: 'workspace-a', read: { status: 'ready', data: ['known'] } };
  slot = beginDomainRead(slot, 'request-3', 'workspace-a');
  slot = failDomainRead(slot, 'request-3', 'workspace-a', 'READ_FAILED');
  assert.deepEqual(visibleData(slot.read), ['known']);
  assert.equal(slot.read.status, 'error');
});

test('service refresh keeps only same-target confirmed display without gaining write authority', () => {
  const ready = {daemon:'running' as const, gateway:'empty' as const, recoveryReady:true};
  let slot: DomainReadSlot<typeof ready> = {requestId:null,targetKey:'a',read:{status:'ready',data:ready}};
  slot = beginDomainRead(slot,'refresh','a');
  assert.deepEqual(homeState.deriveHomeServicePresentation(slot.read),{state:'ready',refreshing:true,checkFailed:false});
  assert.equal(homeState.deriveHomeServiceStatus(slot.read).operational,false);
  const different=beginDomainRead(slot,'switch','b');
  assert.equal(different.read.status,'loading');
  assert.equal(visibleData(different.read),undefined);
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'error',code:'AUTHORITY_LOST',previous:ready}),{state:'unknown',refreshing:false,checkFailed:true});
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'ready',data:{...ready,gateway:'unknown'}}),{state:'unknown',refreshing:false,checkFailed:false});
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'ready',data:{...ready,nativeConnectionLost:true}}),{state:'connection_lost',refreshing:false,checkFailed:false});
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'ready',data:{...ready,daemon:'read_only'}}),{state:'read_only',refreshing:false,checkFailed:false});
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'ready',data:{...ready,recoveryReady:false}}),{state:'unknown',refreshing:false,checkFailed:false});
  assert.deepEqual(homeState.deriveHomeServicePresentation({status:'ready',data:{...ready,gateway:'unavailable'}}),{state:'unavailable',refreshing:false,checkFailed:false});
  assert.equal(homeState.deriveHomeServiceStatus({status:'ready',data:{...ready,daemon:'read_only'}}).operational,false);
  assert.equal(homeState.deriveHomeServiceStatus({status:'ready',data:{...ready,recoveryReady:false}}).operational,false);
  assert.equal(homeState.deriveHomeServiceStatus({status:'ready',data:{...ready,gateway:'unknown'}}).operational,false);
});

test('operation observations never regress within one operation', () => {
  const current = { operationId: 'operation-1', sequence: 7, state: 'running' as const };
  assert.equal(acceptOperation(current, { ...current, sequence: 6 }), current);
  assert.equal(acceptOperation(current, { ...current, sequence: 8, state: 'succeeded' }).sequence, 8);
});

test('accepted is presented as pending rather than success', () => {
  const accepted = operationLabel({ operationId: 'operation-1', sequence: 1, state: 'accepted' }, 'en');
  assert.equal(accepted, 'Accepted, not completed');
  assert.notEqual(accepted, operationLabel({ operationId: 'operation-1', sequence: 2, state: 'succeeded' }, 'en'));
});

test('subscription check A is distinct from final save B and cleanup ownership', () => {
  assert.equal(subscriptionPresentation({ checkId: 'a', state: 'verified', checkedCandidateRevision: 'r2', validationRef: 'v1' }), 'verified-not-saved');
  assert.equal(subscriptionPresentation({ checkId: 'a', state: 'verified', checkedCandidateRevision: 'r2', validationRef: 'v1', saveOperationId: 'operation-b' }), 'saving');
  assert.equal(subscriptionPresentation({ checkId: 'a', state: 'retained', saveOperationId: 'operation-b' }), 'saved-retained');
  assert.equal(subscriptionPresentation({ checkId: 'a', state: 'retained' }), 'unknown');
});

test('candidate fact completeness is distinct from saved and runtime-ready state', () => {
  const pending = { candidateRef: 'candidate/native', candidateRevision: '18446744073709551615', editRevision: '7', checkId: 'check/7', displayName: 'Native API', inputState: 'missing' as const, factState: 'pending_credential' as const, selectableModelCount: 1, issues: ['CREDENTIAL_REQUIRED'] };
  assert.equal(candidateFactPresentation(pending), 'needs-credential');
  assert.notEqual(pending.candidateRevision, pending.editRevision);
  assert.equal(candidateFactPresentation({ ...pending, candidateRevision: '18446744073709551614', inputState: 'provided', factState: 'complete' }), 'facts-complete');
  const reads = emptyReads();
  if (reads.compute.status !== 'ready') throw new Error('fixture must be ready');
  reads.compute.data.candidates.push({ ...pending, candidateRevision: '18446744073709551614', inputState: 'provided', factState: 'complete' });
  assert.equal(deriveHomePrimaryMode(reads), 'first-use');
});

test('no-authentication does not imply free pricing', () => {
  assert.equal(isExplicitlyFree('unknown'), false);
  assert.equal(isExplicitlyFree('partial'), false);
  assert.equal(isExplicitlyFree('free'), true);
});
