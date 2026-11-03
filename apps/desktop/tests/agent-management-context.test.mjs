import assert from 'node:assert/strict';
import test from 'node:test';
import { managementContext, managementAgentId, sharedManagement } from '../src/features/agents/management-context.ts';
import { agentSettingsSpec } from '../src/features/agents/settings-request.ts';
import { agentSupportsCollaboration, agentSupportsManagement } from '../src/features/agents/ecosystems.ts';

const cli = {agent_id:'agent_codex_default',context_id:'context/profile',codex_access:{root_context_id:'context/root',codex_home:'/native/codex'}};
const desktop = {agent_id:'agent_codex_desktop',context_id:'context/desktop',codex_desktop:{source_codex_home:'/native/codex'},settings:{management_skill:{state:'configured',restore_point_ref:'restore/shared'}}};
const web = {agent_id:'agent_dsh_default',context_id:'context/web'};
const dsh = {...desktop,agent_id:'agent_dsh_desktop',context_id:'context/dsh-desktop'};
const entries=[cli,desktop,web,dsh];
const draft={values:{},codexMode:'profile',restoreNativeModel:'',managementContextId:'context/root'};

test('Desktop management uses one shared owner while model settings keep the separate context',()=>{
 for(const [agent,owner] of [[desktop,'context/root'],[dsh,'context/web']]) {
  const context=managementContext(agent,entries);
  assert.equal(context,owner);
  assert.equal(agentSupportsManagement(agent.agent_id),true);
  assert.equal(agentSupportsCollaboration(agent.agent_id),false);
  assert.equal(sharedManagement(agent.agent_id),true);
  const spec=agentSettingsSpec(agent,'management_skill',false,{...draft,managementContextId:context});
  assert.equal(spec.context_id,owner);
  assert.deepEqual(spec.model,{intent:'keep'});
  assert.deepEqual(spec.collaboration,{intent:'keep'});
  assert.deepEqual(spec.management_skill,{intent:'configure',settings:{}});
  const restore=agentSettingsSpec(agent,'management_skill',true,{...draft,managementContextId:context});
  assert.deepEqual(restore.management_skill,{intent:'restore',restore_point_ref:'restore/shared'});
 }
 assert.equal(managementContext(cli,entries),'context/root');
 assert.equal(managementAgentId(desktop.agent_id),cli.agent_id);
 assert.equal(managementAgentId(dsh.agent_id),web.agent_id);
});

test('Desktop management never guesses an unavailable or different-home shared owner',()=>{
 assert.throws(()=>managementContext(desktop,[desktop]),/AGENT_STATUS_UNVERIFIED/);
 assert.throws(()=>managementContext(desktop,[{...cli,codex_access:{...cli.codex_access,codex_home:'/other'}},desktop]),/AGENT_STATUS_UNVERIFIED/);
 assert.throws(()=>managementContext(dsh,[dsh]),/AGENT_STATUS_UNVERIFIED/);
 assert.throws(()=>agentSettingsSpec(desktop,'management_skill',false,{...draft,managementContextId:undefined}),/AGENT_STATUS_UNVERIFIED/);
});
