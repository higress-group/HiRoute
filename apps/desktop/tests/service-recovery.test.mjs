import assert from 'node:assert/strict';
import test from 'node:test';
import {
  beginDomainRead,
  deriveHomeServiceStatus,
  failDomainRead,
  resolveDomainRead,
  visibleData,
} from '../src/features/home/state.ts';

test('failed recovery refresh preserves previous data without claiming a healthy service', () => {
  const running = { daemon: 'running', gateway: 'ready', recoveryReady: true };
  let slot = { requestId: null, targetKey: 'desktop/local', read: { status: 'ready', data: running } };
  assert.deepEqual(deriveHomeServiceStatus(slot.read), { state: 'ready', operational: true });
  slot = beginDomainRead(slot, 'recovery/1', 'desktop/local');
  assert.deepEqual(deriveHomeServiceStatus(slot.read), { state: 'unknown', operational: false });
  slot = failDomainRead(slot, 'recovery/1', 'desktop/local', 'RESIDENT_UNAVAILABLE');
  assert.deepEqual(visibleData(slot.read), running);
  assert.deepEqual(deriveHomeServiceStatus(slot.read), { state: 'unknown', operational: false });
  slot = beginDomainRead(slot, 'recovery/2', 'desktop/local');
  slot = resolveDomainRead(slot, 'recovery/2', 'desktop/local', running);
  assert.deepEqual(deriveHomeServiceStatus(slot.read), { state: 'ready', operational: true });
});

test('current empty, read-only and unrecovered service facts retain their distinct availability', () => {
  const read = data => ({ status: 'ready', data });
  assert.deepEqual(deriveHomeServiceStatus(read({ daemon: 'running', gateway: 'empty', recoveryReady: true })),
    { state: 'ready', operational: true });
  assert.deepEqual(deriveHomeServiceStatus(read({ daemon: 'running', gateway: 'ready', recoveryReady: false })),
    { state: 'ready', operational: false });
  assert.deepEqual(deriveHomeServiceStatus(read({ daemon: 'read_only', gateway: 'ready', recoveryReady: true })),
    { state: 'read_only', operational: false });
  assert.deepEqual(deriveHomeServiceStatus(read({ daemon: 'unavailable', gateway: 'unknown', recoveryReady: false })),
    { state: 'unavailable', operational: false });
  assert.deepEqual(deriveHomeServiceStatus(read({ daemon: 'running', gateway: 'ready', recoveryReady: true, nativeConnectionLost: true })),
    { state: 'connection_lost', operational: false });
});
