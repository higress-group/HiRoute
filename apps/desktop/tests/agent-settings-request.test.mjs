import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentSettingsSpec, agentTokenSpec, prerequisiteCheck } from '../src/features/agents/settings-request.ts';
import { agentEditorSeed, agentModelFormInvalid, EMPTY_EDITOR_VALUES } from '../src/features/agents/editor-state.ts';

const codex = {
  agent_id: 'agent_codex_default', context_id: 'context/current', status_error: null,
  settings: { state: 'configured', restore_point_ref: 'restore/model', collaboration: { restore_point_ref: 'restore/skill' } },
  codex_access: {
    slot_occupied: false, selected_mode: 'profile',
    profile_context_id: 'context/profile', root_context_id: 'context/root',
  },
};
const claude = { ...codex, agent_id: 'agent_claude_default', codex_access: undefined };
const qoder = {
  agent_id: 'agent_qoder_default', context_id: 'context/qoder', status_error: null,
  settings: {
    schema: 'hiroute.agent-collaboration-only-settings-status/v2', context_id: 'context/qoder',
    collaboration: { state: 'configured', restore_point_ref: 'restore/qoder-skill', current_selection: { trigger_mode: 'explicit' } },
  },
};
const fixed = { client_model_id: 'native-a', candidate: { binding_id: 'binding/account-a', reasoning: { kind: 'profile', profile: 'high' } } };
const draft = {
  values: {
    ...EMPTY_EDITOR_VALUES, fixedModels: [fixed], nativeModelMode: 'preserve_available',
    allowedPlanIds: ['route/b', 'route/a'], defaultChoice: { kind: 'plan', plan_id: 'route/b' },
    claudePresets: { opus: { kind: 'plan', plan_id: 'route/a' }, sonnet: { kind: 'preserve_native' }, haiku: { kind: 'plan', plan_id: 'route/b' } },
    triggerMode: 'delegate_by_default',
  },
  codexMode: 'profile', restoreNativeModel: 'native-on-restore',
};
const wire = value => JSON.parse(JSON.stringify(value));

test('Codex configuration submits its exact allowlist, default and account-bound fixed model without changing collaboration', () => {
  const before = structuredClone({ codex, draft });
  assert.deepEqual(wire(agentSettingsSpec(codex, 'model', false, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/profile',
    model: { intent: 'configure', settings: {
      mode: 'codex_default', native_model_mode: 'preserve_available', fixed_models: [fixed],
      allowed_plan_ids: ['route/b', 'route/a'], default_selection: { kind: 'plan', plan_id: 'route/b' },
    } },
    collaboration: { intent: 'keep' },
  });
  assert.deepEqual({ codex, draft }, before);
});

test('new Codex access targets the chosen mode while an occupied slot retains its current context', () => {
  assert.equal(agentSettingsSpec(codex, 'model', false, { ...draft, codexMode: 'root' }).context_id, 'context/root');
  const occupied = { ...codex, codex_access: { ...codex.codex_access, slot_occupied: true } };
  assert.equal(agentSettingsSpec(occupied, 'model', false, { ...draft, codexMode: 'root' }).context_id, 'context/current');
});

test('Claude configuration preserves independent native preset mappings and does not acquire a Codex default or allowlist', () => {
  assert.deepEqual(wire(agentSettingsSpec(claude, 'model', false, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/current',
    model: { intent: 'configure', settings: {
      mode: 'claude_launcher', surfaces: ['claude_cli'], fixed_models: [fixed],
      preset_mappings: { opus: { kind: 'plan', plan_id: 'route/a' }, sonnet: { kind: 'preserve_native' }, haiku: { kind: 'plan', plan_id: 'route/b' } },
    } },
    collaboration: { intent: 'keep' },
  });
});

test('ordinary disable requests restoration from the saved reference and keeps model and collaboration recovery separate', () => {
  const profile = wire(agentSettingsSpec(codex, 'model', true, draft));
  assert.deepEqual(profile, {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/profile',
    model: { intent: 'restore', restore_point_ref: 'restore/model' }, collaboration: { intent: 'keep' },
  });
  const root = { ...codex, codex_access: { ...codex.codex_access, slot_occupied: true, selected_mode: 'root' } };
  assert.equal(agentSettingsSpec(root, 'model', true, draft).restore_native_model, 'native-on-restore');
  assert.equal(agentSettingsSpec(claude, 'model', true, draft).restore_native_model, undefined);
  assert.deepEqual(wire(agentSettingsSpec(codex, 'collaboration', true, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/root',
    model: { intent: 'keep' }, collaboration: { intent: 'restore', restore_point_ref: 'restore/skill' },
  });
});

test('enabling task routing sends only the trigger preference, independently of the model draft', () => {
  assert.deepEqual(wire(agentSettingsSpec(codex, 'collaboration', false, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/root', model: { intent: 'keep' },
    collaboration: { intent: 'configure', settings: { trigger_mode: 'delegate_by_default' } },
  });
});

test('Qoder configures either collaboration trigger and restores only its Skill without a model facet', () => {
  const before = structuredClone({ qoder, draft });
  for (const triggerMode of ['explicit', 'delegate_by_default']) {
    assert.deepEqual(wire(agentSettingsSpec(qoder, 'collaboration', false, {
      ...draft, values: { ...draft.values, triggerMode },
    })), {
      schema_version: { major: 2, minor: 0 }, context_id: 'context/qoder',
      model: { intent: 'keep' },
      collaboration: { intent: 'configure', settings: { trigger_mode: triggerMode } },
    });
  }
  assert.deepEqual(wire(agentSettingsSpec(qoder, 'collaboration', true, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: 'context/qoder',
    model: { intent: 'keep' },
    collaboration: { intent: 'restore', restore_point_ref: 'restore/qoder-skill' },
  });
  assert.deepEqual({ qoder, draft }, before);
});

test('collaboration-only Qoder and cross-ecosystem selections cannot authorize models or tokens', () => {
  const stale = {
    ...qoder,
    settings: { ...claude.settings, current_selection: agentSettingsSpec(claude, 'model', false, draft).model.settings },
  };
  for (const agent of [qoder, stale]) {
    for (const restore of [false, true]) {
      assert.throws(() => agentSettingsSpec(agent, 'model', restore, draft), /AGENT_INPUT_INVALID/);
    }
    for (const regenerate of [false, true]) {
      assert.throws(() => agentTokenSpec(agent, regenerate), /AGENT_INPUT_INVALID/);
    }
  }
});

for (const ecosystem of ['qoder', 'pi', 'dsh']) test(`${ecosystem} adds only selected routes and keeps token rotation and both restores independent`, () => {
  const native = { ...qoder, agent_id: `agent_${ecosystem}_default`, context_id: `context/${ecosystem}`,
    settings: { collaboration: { ...qoder.settings.collaboration, restore_point_ref: `restore/${ecosystem}-skill` } } };
  const capable = { ...native, settings: { state: 'not_configured', collaboration: native.settings.collaboration } };
  const configure = wire(agentSettingsSpec(capable, 'model', false, draft));
  assert.deepEqual(configure, {
    schema_version: { major: 2, minor: 0 }, context_id: `context/${ecosystem}`,
    model: { intent: 'configure', settings: { mode: `${ecosystem}_additional`, allowed_plan_ids: ['route/b', 'route/a'], plan_protocols: { 'route/b': 'responses', 'route/a': 'responses' } } },
    collaboration: { intent: 'keep' },
  });
  const saved = { ...capable, settings: {
    state: 'configured', current_selection: configure.model.settings, restore_point_ref: `restore/${ecosystem}-model`,
    collaboration: native.settings.collaboration,
  } };
  assert.deepEqual(agentTokenSpec(saved, true), {
    schema_version: { major: 2, minor: 0 }, context_id: `context/${ecosystem}`,
    model: configure.model, collaboration: { intent: 'keep' }, access_token: { intent: 'regenerate' },
  });
  assert.deepEqual(wire(agentSettingsSpec(saved, 'model', true, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: `context/${ecosystem}`,
    model: { intent: 'restore', restore_point_ref: `restore/${ecosystem}-model` }, collaboration: { intent: 'keep' },
  });
  assert.deepEqual(wire(agentSettingsSpec(saved, 'collaboration', true, draft)), {
    schema_version: { major: 2, minor: 0 }, context_id: `context/${ecosystem}`,
    model: { intent: 'keep' }, collaboration: { intent: 'restore', restore_point_ref: `restore/${ecosystem}-skill` },
  });
  for (const agent of [codex, claude]) {
    const mixed = { ...agent, settings: saved.settings };
    assert.equal(agentEditorSeed(mixed, 'model').known, false);
    assert.throws(() => agentSettingsSpec(mixed, 'model', false, draft), /AGENT_INPUT_INVALID/);
    assert.throws(() => agentTokenSpec(mixed, true), /AGENT_INPUT_INVALID/);
  }
});

test('token rotation retains the saved selection without consulting an unsaved model draft', () => {
  const saved = { ...claude, settings: { ...claude.settings, current_selection: agentSettingsSpec(claude, 'model', false, draft).model.settings } };
  for (const regenerate of [false, true]) {
    assert.deepEqual(agentTokenSpec(saved, regenerate), {
      schema_version: { major: 2, minor: 0 }, context_id: 'context/current',
      model: { intent: 'configure', settings: saved.settings.current_selection },
      collaboration: { intent: 'keep' }, access_token: { intent: regenerate ? 'regenerate' : 'keep' },
    });
  }
});

test('a discovered but unsupported ecosystem never borrows the Claude editor or produces a settings intent', () => {
  const configured = agentSettingsSpec(claude, 'model', false, draft).model.settings;
  for (const current_selection of [null, configured]) {
    const unknown = { ...claude, agent_id: 'agent_unregistered', settings: { state: current_selection ? 'configured' : 'not_configured', current_selection } };
    assert.equal(agentEditorSeed(unknown, 'model', ['only-route']).known, false);
    assert.equal(agentModelFormInvalid(unknown, draft.values, ['route/a', 'route/b']), true);
    for (const facet of ['model', 'collaboration']) {
      for (const restore of [false, true]) assert.throws(() => agentSettingsSpec(unknown, facet, restore, draft), /AGENT_INPUT_INVALID/);
    }
    assert.throws(() => agentTokenSpec(unknown, true), /AGENT_INPUT_INVALID/);
  }
  assert.throws(() => agentSettingsSpec({ ...codex, context_id: null }, 'model', false, draft), /AGENT_INPUT_INVALID/);
});

test('pre-save checks follow authoritative capability blockers rather than the ecosystem or the visible form', () => {
  const blocked = (...capabilities) => ({ applicable: false, blockers: [{ reason: 'capability', capabilities: capabilities.map(capability => ({ capability, reason: 'unconfirmed' })) }] });
  for (const facet of ['model', 'collaboration']) {
    assert.equal(prerequisiteCheck(blocked('ingress_authentication', 'skill_loading'), facet), 'native_authentication');
    assert.equal(prerequisiteCheck({ ...blocked('ingress_authentication'), applicable: true }, facet), null);
  }
  assert.equal(prerequisiteCheck(blocked('skill_loading'), 'collaboration'), 'collaboration');
  assert.equal(prerequisiteCheck(blocked('trusted_cli_execution'), 'collaboration'), 'collaboration');
  assert.equal(prerequisiteCheck(blocked('skill_loading'), 'model'), null);
  assert.equal(prerequisiteCheck(blocked('unknown_capability'), 'model'), null);
});

test('editing model routes retains distinct Codex authorization and Claude preset requirements', () => {
  const initial = agent => ({ ...agent, settings: { state: 'not_configured', current_selection: null } });
  for (const agent of [initial(codex), initial(claude)]) {
    const values = agentEditorSeed(agent, 'model', ['only-route']);
    assert.equal(agentModelFormInvalid(agent, values, ['only-route']), false);
    assert.equal(agentModelFormInvalid(agent, values, []), true, 'a route removed while editing must block Save');
  }
  const retained = { ...codex, settings: { ...codex.settings, protected_native_model_ids: ['native-a'] } };
  assert.equal(agentModelFormInvalid(retained, draft.values, ['route/a', 'route/b']), false, 'a protected binding does not need reselection');
  assert.equal(agentModelFormInvalid(codex, draft.values, ['route/a', 'route/b']), true, 'an unproven independent fixed source must block Save');
  assert.equal(agentModelFormInvalid(claude, { ...EMPTY_EDITOR_VALUES, fixedModels: [fixed] }, []), false, 'an existing Claude fixed model can survive without a preset route');
});

for (const ecosystem of ['qoder', 'pi', 'dsh']) test(`${ecosystem} saves each selected plan protocol and excludes deselected plans`, () => {
  const capable = { ...qoder, agent_id: `agent_${ecosystem}_default`, context_id: `context/${ecosystem}`, settings: { state: 'not_configured' } };
  const values = { ...draft.values, planProtocols: { 'route/a': 'messages', 'route/b': 'responses', 'removed': 'messages' } };
  const result = agentSettingsSpec(capable, 'model', false, { ...draft, values });
  assert.deepEqual(result.model.settings.plan_protocols, { 'route/b': 'responses', 'route/a': 'messages' });
  assert.deepEqual(result.collaboration, { intent: 'keep' });
});
