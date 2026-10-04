import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentEditorSeed, codexDefaultChoiceValid, commonClaudePlan, sharedClaudePlan, editorFingerprint } from '../src/agent-editor-state.ts';
import { agentModelFormInvalid } from '../src/features/agents/editor-state.ts';
import { agentModelStatus } from '../src/features/agents/status.ts';

const codexSelection = {
  mode: 'codex_default',
  native_model_mode: 'preserve_available',
  fixed_models: [{ client_model_id: 'native-model', candidate: { binding_id: 'binding/native' } }],
  allowed_plan_ids: ['a', 'b'],
  default_selection: { kind: 'plan', plan_id: 'a' },
};
const codex = {
  agent_id: 'agent_codex_default', configuration_state: 'configured', status_error: null,
  available_surfaces: ['codex_desktop', 'codex_cli'],
  settings: {
    state: 'configured', current_selection: codexSelection,
    collaboration: { state: 'configured', current_selection: { trigger_mode: 'delegate_by_default' } },
  },
};
const claude = {
  agent_id: 'agent_claude_default', configuration_state: 'configured', status_error: null,
  settings: {
    state: 'configured',
    current_selection: {
      mode: 'claude_launcher', surfaces: ['claude_cli'], fixed_models: [],
      preset_mappings: {
        opus: { kind: 'plan', plan_id: 'a' },
        sonnet: { kind: 'plan', plan_id: 'a' },
        haiku: { kind: 'preserve_native' },
      },
    },
  },
};

test('Codex seed preserves the shared allowlist and default choice', () => {
  const value = agentEditorSeed(codex, 'model');
  assert.equal(value.known, true);
  assert.deepEqual(value.allowedPlanIds, ['a', 'b']);
  assert.deepEqual(value.defaultChoice, { kind: 'plan', plan_id: 'a' });
  assert.equal(value.nativeModelMode, 'preserve_available');
  assert.equal(value.fixedModels[0].client_model_id, 'native-model');
});

test('Claude seed keeps exactly three independent mappings and permits repeats', () => {
  const value = agentEditorSeed(claude, 'model');
  assert.equal(value.known, true);
  assert.deepEqual(value.claudePresets.opus, value.claudePresets.sonnet);
  assert.deepEqual(value.claudePresets.haiku, { kind: 'preserve_native' });
});

test('collaboration retains only its task trigger mode', () => {
  const value = agentEditorSeed(codex, 'collaboration');
  assert.equal(value.known, true);
  assert.equal(value.triggerMode, 'delegate_by_default');
});

test('Qoder collaboration-only status seeds its trigger without making a model editor or selecting model routes', () => {
  for (const collaboration of [
    { state: 'not_configured', restore_point_ref: null, current_selection: null },
    { state: 'configured', restore_point_ref: 'restore/qoder-skill', current_selection: { trigger_mode: 'explicit' } },
    { state: 'configured', restore_point_ref: 'restore/qoder-skill', current_selection: { trigger_mode: 'delegate_by_default' } },
  ]) {
    const qoder = {
      agent_id: 'agent_qoder_default', context_id: 'context/qoder', status_error: null,
      settings: {
        schema: 'hiroute.agent-collaboration-only-settings-status/v2', context_id: 'context/qoder', collaboration,
      },
    };
    assert.equal(agentModelStatus(qoder), null);
    const model = agentEditorSeed(qoder, 'model', ['only-route']);
    assert.equal(model.known, false);
    assert.deepEqual(model.allowedPlanIds, []);
    assert.deepEqual(model.fixedModels, []);
    assert.equal(commonClaudePlan(model.claudePresets), '');
    const seed = agentEditorSeed(qoder, 'collaboration', ['only-route']);
    assert.equal(seed.known, true);
    assert.equal(seed.triggerMode, collaboration.current_selection?.trigger_mode ?? 'explicit');
    assert.deepEqual(seed.allowedPlanIds, []);
  }
});

test('unknown, drift and unavailable status cannot become empty authorization', () => {
  for (const state of ['configured', 'drift', 'pending', 'needs_attention']) {
    assert.equal(agentEditorSeed({ ...codex, settings: { state } }, 'model').known, false);
  }
  assert.equal(agentEditorSeed({ ...codex, status_error: 'UNAVAILABLE' }, 'model').known, false);
});

test('Qoder preserves route selections without a native catalog and retains unavailable choices for repair', () => {
  const agent = { agent_id: 'agent_qoder_default', status_error: null, native_model_catalog: null,
    settings: { state: 'not_configured', current_selection: null } };
  const initial = agentEditorSeed(agent, 'model', ['only-route']);
  assert.equal(initial.known, true);
  assert.deepEqual(initial.allowedPlanIds, ['only-route']);
  assert.deepEqual(initial.fixedModels, []);
  assert.deepEqual(initial.defaultChoice, { kind: 'preserve_native' });
  assert.equal(agentModelFormInvalid(agent, initial, ['only-route']), false);
  assert.equal(agentModelFormInvalid(agent, initial, []), true);
  const saved = { ...agent, settings: { state: 'configured', current_selection: {
    mode: 'qoder_additional', allowed_plan_ids: ['route/retired', 'route/current'],
  } } };
  const seed = agentEditorSeed(saved, 'model', ['route/new']);
  assert.equal(seed.known, true);
  assert.deepEqual(seed.allowedPlanIds, ['route/retired', 'route/current']);
  assert.equal(agentModelFormInvalid(saved, seed, ['route/current']), true);
  const repaired = { ...seed, allowedPlanIds: ['route/current'] };
  assert.equal(agentModelFormInvalid(saved, repaired, ['route/current']), false);
  assert.notEqual(editorFingerprint(seed), editorFingerprint(repaired));
});

test('Codex discovery facts do not change the initial shared configuration', () => {
  const desktop = agentEditorSeed({ ...codex, available_surfaces: ['codex_desktop'], settings: { state: 'not_configured' } }, 'model');
  const cli = agentEditorSeed({
    ...codex,
    available_surfaces: ['codex_cli'],
    settings: { state: 'not_configured' },
  }, 'model');
  assert.equal(desktop.known, true);
  assert.deepEqual(desktop, cli);
  assert.deepEqual(desktop.allowedPlanIds, []);
  assert.equal(desktop.nativeModelMode, 'hiroute_only');
  assert.deepEqual(desktop.defaultChoice, { kind: 'preserve_native' });
});

test('preserving the current Codex default leaves route coverage to the backend', () => {
  assert.equal(
    codexDefaultChoiceValid({ kind: 'preserve_native' }, 'preserve_available', 'gpt-6-astra', [], ['plan/translate']),
    true,
  );
  assert.equal(
    codexDefaultChoiceValid({ kind: 'preserve_native' }, 'preserve_available', undefined, [], ['plan/translate']),
    false,
  );
  assert.equal(
    codexDefaultChoiceValid({ kind: 'plan', plan_id: 'plan/translate' }, 'hiroute_only', 'gpt-6-astra', [], []),
    false,
  );
  assert.equal(codexDefaultChoiceValid({ kind: 'preserve_native' }, 'hiroute_only', 'gpt-6-astra', [], ['plan/translate']), false);
});

test('configured collaboration requires an explicit current selection', () => {
  const missing = structuredClone(codex);
  missing.settings.collaboration.current_selection = null;
  assert.equal(agentEditorSeed(missing, 'collaboration').known, false);
});

test('all model and collaboration choices participate in dirty tracking', () => {
  const { known, ...values } = agentEditorSeed(codex, 'model');
  assert.equal(known, true);
  const edits = [
    { triggerMode: 'delegate_by_default' },
    { nativeModelMode: 'hiroute_only' },
    { allowedPlanIds: ['a'] },
    { defaultChoice: { kind: 'fixed_model', client_model_id: 'native-model' } },
    { fixedModels: [{ client_model_id: 'native-model', candidate: { binding_id: 'binding/other-account' } }] },
    { fixedModels: [{ client_model_id: 'native-model', candidate: { binding_id: 'binding/native', reasoning: { kind: 'toggle', enabled: false } } }] },
    { claudePresets: { ...values.claudePresets, haiku: { kind: 'plan', plan_id: 'a' } } },
  ];
  for (const edit of edits) {
    assert.notEqual(editorFingerprint(values), editorFingerprint({ ...values, ...edit }), JSON.stringify(edit));
  }
  assert.equal(
    editorFingerprint(values),
    editorFingerprint({ ...values, allowedPlanIds: [...values.allowedPlanIds].reverse() }),
  );
});

test('first enable selects the sole route and a valid Codex default without changing the Agent', () => {
  const fresh = { ...codex, settings: { state: 'not_configured', current_selection: null } };
  const before = structuredClone(fresh);
  const seed = agentEditorSeed(fresh, 'model', ['only-route']);
  assert.deepEqual(seed.allowedPlanIds, ['only-route']);
  assert.deepEqual(seed.defaultChoice, { kind: 'plan', plan_id: 'only-route' });
  assert.equal(codexDefaultChoiceValid(seed.defaultChoice, seed.nativeModelMode, undefined, [], seed.allowedPlanIds), true);
  assert.deepEqual(fresh, before);
});

test('first Claude enable maps the sole route to all three presets without setting a native default', () => {
  const fresh = { ...claude, settings: { state: 'restored', current_selection: null } };
  const seed = agentEditorSeed(fresh, 'model', ['only-route']);
  assert.deepEqual(seed.claudePresets, sharedClaudePlan('only-route'));
  assert.equal(commonClaudePlan(seed.claudePresets), 'only-route');
  assert.deepEqual(seed.defaultChoice, { kind: 'preserve_native' });
});

test('zero or multiple routes require selection and do not silently authorize a route', () => {
  for (const agent of [codex, claude]) {
    const fresh = { ...agent, settings: { state: 'not_configured', current_selection: null } };
    for (const choices of [[], ['a', 'b']]) {
      const seed = agentEditorSeed(fresh, 'model', choices);
      assert.deepEqual(seed.allowedPlanIds, []);
      assert.equal(commonClaudePlan(seed.claudePresets), '');
      assert.deepEqual(seed.defaultChoice, { kind: 'preserve_native' });
    }
  }
});

test('editing existing, unavailable or split mappings never applies first-enable shortcuts', () => {
  assert.deepEqual(agentEditorSeed(codex, 'model', ['replacement']).allowedPlanIds, ['a', 'b']);
  const oldClaude = agentEditorSeed(claude, 'model', ['replacement']);
  assert.deepEqual(oldClaude.claudePresets, claude.settings.current_selection.preset_mappings);
  assert.equal(commonClaudePlan(oldClaude.claudePresets), null);
  assert.equal(commonClaudePlan(sharedClaudePlan('missing-route')), 'missing-route');
  for (const state of ['drift', 'pending', 'needs_attention']) {
    const seed = agentEditorSeed({ ...codex, settings: { state, current_selection: null } }, 'model', ['replacement']);
    assert.equal(seed.known, false);
    assert.deepEqual(seed.allowedPlanIds, []);
  }
});

test('collaboration and unreadable initial settings do not pick model routes', () => {
  assert.deepEqual(agentEditorSeed(codex, 'collaboration', ['only-route']).allowedPlanIds, []);
  const seed = agentEditorSeed({ ...codex, status_error: 'UNAVAILABLE', settings: { state: 'not_configured' } }, 'model', ['only-route']);
  assert.equal(seed.known, false);
  assert.deepEqual(seed.allowedPlanIds, []);
});

test('an existing model selection from another ecosystem cannot become an editable draft', () => {
  for (const [agent, other] of [[codex, claude], [claude, codex]]) {
    const seed = agentEditorSeed({
      ...agent,
      settings: { ...agent.settings, current_selection: other.settings.current_selection },
    }, 'model', ['replacement']);
    assert.equal(seed.known, false);
  }
});
