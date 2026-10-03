import { test } from 'node:test';
import assert from 'node:assert/strict';
import { codexSurfaceFacts } from '../src/features/agents/codex-surfaces.ts';

test('Desktop discovery persists while CLI profile applicability changes with the mode', () => {
  const installed = ['codex_desktop', 'codex_cli'];
  const profile = codexSurfaceFacts('profile', installed);
  const root = codexSurfaceFacts('root', installed);
  assert.deepEqual(profile.filter(f => f.detected).map(f => f.surface), installed);
  assert.deepEqual(root.filter(f => f.detected).map(f => f.surface), installed);
  assert.deepEqual(profile.filter(f => f.applicable).map(f => f.surface), ['codex_cli']);
  assert.deepEqual(root.filter(f => f.applicable).map(f => f.surface), installed);
});

test('mode selection never invents an executable and does not remove an installed Desktop', () => {
  assert.deepEqual(codexSurfaceFacts('profile', ['codex_desktop']), [
    { surface: 'codex_desktop', detected: true, applicable: false },
    { surface: 'codex_cli', detected: false, applicable: true },
  ]);
});
