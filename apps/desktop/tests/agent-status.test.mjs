import assert from 'node:assert/strict';
import test from 'node:test';
import { projectHomeAgents } from '../src/product/home-projections.ts';
import { deriveHomePrimaryMode } from '../src/features/home/state.ts';

test('a collaboration-only connection does not invent a missing model setup or count as configured before enable', () => {
  const agent = {
    agent_id: 'agent_qoder_default', context_id: 'context/native', configuration_state: 'configured',
    settings: { schema: 'hiroute.agent-collaboration-only-settings-status/v2', context_id: 'context/native',
      collaboration: { state: 'not_configured', restore_point_ref: null } },
  };
  const project = value => projectHomeAgents({ agents: [value] });
  const projected = project(agent);
  assert.equal(projected.agents[0].model, 'unsupported');
  assert.equal(projected.agents[0].collaboration, 'unconfigured');
  assert.equal(project({ ...agent, agent_id: 'another-collaboration-only-agent' }).agents[0].model, 'unsupported');
  assert.equal(project({ ...agent, settings: null }).agents[0].model, 'unknown');
  const reads = {
    compute: { status: 'ready', data: { sources: [], saveAttempts: [] } },
    plans: { status: 'ready', data: { plans: [], drafts: [] } },
    agents: { status: 'ready', data: projected },
    activity: { status: 'ready', data: { sessions: [], tasks: [] } },
  };
  assert.equal(deriveHomePrimaryMode(reads), 'first-use');
  agent.settings.collaboration = { state: 'configured', restore_point_ref: 'restore/skill', current_selection: { trigger_mode: 'explicit' } };
  reads.agents.data = project(agent);
  assert.equal(reads.agents.data.agents[0].model, 'unsupported');
  assert.equal(reads.agents.data.agents[0].collaboration, 'configured');
  assert.equal(deriveHomePrimaryMode(reads), 'partial');
});

test('Agent presentation follows configuration and recovery rather than model verification', () => {
  const agent = { agent_id: 'agent_qoder_default', context_id: 'context/native', configuration_state: 'configured',
    settings: { state: 'not_configured', model_verified: false,
      collaboration: { state: 'configured', restore_point_ref: 'restore/skill' } } };
  const project = () => projectHomeAgents({ agents: [agent] }).agents[0];
  assert.equal(project().model, 'unconfigured');
  assert.equal(project().collaboration, 'configured');
  agent.settings.state = 'configured';
  assert.equal(project().model, 'configured');
  agent.settings.model_verified = true;
  assert.equal(project().model, 'configured');
  agent.settings.state = 'drift';
  assert.equal(project().model, 'degraded');
  agent.settings.state = 'pending';
  assert.equal(project().model, 'pending');
});
