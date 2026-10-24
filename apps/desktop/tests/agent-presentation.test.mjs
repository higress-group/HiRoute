import assert from 'node:assert/strict';
import test from 'node:test';
import { agentDisplayName } from '../src/features/agents/ecosystems.ts';
import { projectHomeActivity, projectHomeAgents } from '../src/product/home-projections.ts';

const integrations = [
  ['dsh', 'DeepSeek Harness', 'dsh'], ['codex', 'Codex', 'codex'],
  ['claude', 'Claude Code', 'claude-code'], ['pi', 'Pi', 'pi'],
  ['qoder', 'Qoder', 'qoder'], ['hermes', 'Hermes', 'hermes'],
  ['opencode', 'OpenCode', 'opencode'],
];

test('supported Agent names survive Home configuration and recent-session projections', () => {
  for (const [family, name, brand] of integrations) {
    const agent_id = `agent_${family}_default`;
    for (const language of ['zh', 'en']) assert.equal(agentDisplayName(agent_id, language), name);
    const configured = projectHomeAgents({ agents: [{ agent_id, settings: null }] }).agents[0];
    assert.equal(configured.displayName, name);
    assert.equal(configured.brand, brand);
    const recent = projectHomeActivity({ sessions: [{
      session_id: `session/${family}`, agent_id, first_request_at_ms: 1,
      last_request_at_ms: 2, request_count: 1, fallback_request_count: 0,
      unknown_model_request_count: 0, correlation_kind: 'agent_supplied',
    }], next_cursor: null }).sessions[0];
    assert.equal(recent.agentName, name);
    assert.equal(recent.agentId, agent_id);
  }
});

test('unknown and unlinked identities do not acquire a supported Agent name', () => {
  assert.equal(agentDisplayName('agent_unknown_default', 'zh'), '本机 Agent');
  assert.equal(agentDisplayName('agent_unknown_default', 'en'), 'Local Agent');
  const recent = projectHomeActivity({ sessions: [{
    session_id: 'session/unlinked', agent_id: '', first_request_at_ms: 1,
    last_request_at_ms: 2, request_count: 1, fallback_request_count: 0,
    unknown_model_request_count: 0, correlation_kind: 'unknown',
  }], next_cursor: null }).sessions[0];
  assert.equal(recent.agentName, undefined);
});
