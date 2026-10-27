import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createTrialAlias, currentPrimary, trialMappingValues, initialTrialMapping, trialSelection } from '../src/features/trial/model.ts';
import { agentEditorSeed, editorFingerprint } from '../src/features/agents/editor-state.ts';
import { selectedTrialSnapshot, trialModelChoices } from '../src/features/trial/agent-read.ts';
const native = { agent_id:'agent_claude_default', context_id:'claude/context', status_error:null, settings:{state:'not_configured'} };
const configured = { ...native, settings:{state:'configured',current_selection:{mode:'claude_launcher',surfaces:['claude_cli'],fixed_models:[],preset_mappings:{opus:{kind:'plan',plan_id:'primary'},sonnet:{kind:'plan',plan_id:'separate'},haiku:{kind:'preserve_native'}}}} };
test('discovered choices never seed empty settings and mismatched detail contexts are rejected',()=>{
 const {settings:_,status_error:__,...choice}=native;
 assert.equal(agentEditorSeed(choice,'model',[]).known,false);
 assert.throws(()=>trialMappingValues(choice,'trial',{agentId:choice.agent_id,presets:['opus']}),/AGENT_SELECTION_UNKNOWN/);
 assert.deepEqual(trialModelChoices([choice,{...choice,context_id:null},{...choice,agent_id:'agent_hermes_default'}]),[choice]);
 assert.throws(()=>selectedTrialSnapshot({choices:[choice],trusted_authority:true},choice,{settings:{...native.settings,context_id:'different'},plans:{plans:[]},trusted_authority:true}),/AGENT_STATUS_UNAVAILABLE/);
 const pending=selectedTrialSnapshot({choices:[choice],trusted_authority:true},choice,{settings:{state:'pending',context_id:choice.context_id},plans:{plans:[]},trusted_authority:true});
 assert.equal(agentEditorSeed(pending.selected,'model',[]).known,false);
});
for(const ecosystem of ['dsh','pi','qoder','opencode']) test(`${ecosystem} trial appends its route while retaining existing protocols`,()=>{
 const agent={...native,agent_id:`agent_${ecosystem}_default`,context_id:`${ecosystem}/context`,settings:{state:'configured',current_selection:{mode:`${ecosystem}_additional`,allowed_plan_ids:['original'],plan_protocols:{original:'messages'}}}};
 const before=structuredClone(agent);
 const values=trialMappingValues(agent,'trial',{agentId:agent.agent_id,presets:[]});
 assert.deepEqual(values.allowedPlanIds,['original','trial']);
 assert.deepEqual(values.planProtocols,{original:'messages',trial:'responses'});
 assert.deepEqual(agent,before);
});
test('first Claude trial maps only the explicitly selected Opus preset',()=>{
 const values=trialMappingValues(native,'trial',{agentId:native.agent_id,presets:['opus']});
 assert.deepEqual(values.claudePresets,{opus:{kind:'plan',plan_id:'trial'},sonnet:{kind:'preserve_native'},haiku:{kind:'preserve_native'}});
});
test('trial preserves a separately configured Sonnet and all fixed-model bindings',()=>{
 const values=trialMappingValues(configured,'trial',{agentId:configured.agent_id,presets:['opus']});
 assert.deepEqual(values.claudePresets.sonnet,configured.settings.current_selection.preset_mappings.sonnet);
 assert.deepEqual(configured.settings.current_selection.preset_mappings.opus,{kind:'plan',plan_id:'primary'});
 assert.throws(()=>trialMappingValues({...configured,status_error:'unavailable'},'trial',{agentId:configured.agent_id,presets:['opus']}));
});
test('confirmed native snapshot matches a trial mapping despite JSON object key order',()=>{
 const requested=trialMappingValues(configured,'trial',{agentId:configured.agent_id,presets:['opus']});
 // The native JSON boundary returns map entries in key order, unlike the form.
 const confirmed={...configured,settings:{...configured.settings,current_selection:{...configured.settings.current_selection,preset_mappings:{
  haiku:{kind:'preserve_native'},opus:{plan_id:'trial',kind:'plan'},sonnet:{plan_id:'separate',kind:'plan'},
 }}}};
 assert.equal(editorFingerprint(agentEditorSeed(confirmed,'model',[])),editorFingerprint(requested));
 const changed=structuredClone(confirmed);changed.settings.current_selection.preset_mappings.sonnet.plan_id='other';
 assert.notEqual(editorFingerprint(agentEditorSeed(changed,'model',[])),editorFingerprint(requested));
 const {known,...values}=requested;
 assert.equal(known,true);
 assert.equal(editorFingerprint(requested),editorFingerprint(values));
});
test('Primary defaults only from a unique enabled fixed-plan resolution, never a native alias',()=>{
 const plan={agent_plan_id:'primary',head:{status:'enabled'},desired:{mode:'fixed_model',strategy:{candidates:[{binding_id:'b',reasoning:{kind:'profile',profile:'high'}}]}}};
 assert.equal(currentPrimary(native,[plan]),undefined);
 assert.deepEqual(currentPrimary(configured,[plan]),plan.desired.strategy.candidates[0]);
 assert.equal(currentPrimary(configured,[{...plan,head:{status:'disabled'}}]),undefined);
 assert.equal(currentPrimary(configured,[{...plan,desired:{...plan.desired,strategy:{candidates:[{binding_id:'a'},{binding_id:'b'}]}}}]),undefined);
 assert.deepEqual(initialTrialMapping({choices:[native],selected:native,plans:{plans:[]}}),{agentId:'agent_claude_default',presets:['opus']});
});

test('trial reasoning uses only the selected binding default and retains explicit choices',()=>{
 const candidate={binding_id:'flash',reasoning:{kind:'discrete',parameter:'effort',profiles:['low','high','max'],default_profile:'high'}};
 assert.deepEqual(trialSelection(candidate),{state:'selected',selection:{binding_id:'flash',reasoning:{kind:'profile',profile:'high'}}});
 assert.deepEqual(trialSelection(candidate,{binding_id:'flash',reasoning:{kind:'profile',profile:'low'}}),{state:'selected',selection:{binding_id:'flash',reasoning:{kind:'profile',profile:'low'}}});
 assert.equal(trialSelection({...candidate,reasoning:{...candidate.reasoning,default_profile:undefined}}).state,'choice_required');
 assert.equal(trialSelection({...candidate,reasoning:{...candidate.reasoning,default_profile:'unsupported'}}).state,'invalid');
 assert.equal(trialSelection(candidate,{binding_id:'flash',reasoning:{kind:'profile',profile:'removed'}}).state,'invalid');
 assert.equal(trialSelection({...candidate,reasoning:{kind:'fixed',profile:'fixed'}}).state,'selected');
 assert.equal(trialSelection({...candidate,reasoning:{kind:'toggle',parameter:'thinking'}}).state,'choice_required');
 assert.equal(trialSelection({...candidate,reasoning:{kind:'budget',parameter:'budget',minimum_tokens:10,maximum_tokens:100,step_tokens:10}}).state,'choice_required');
});

test('a DSH allowlist is not evidence of its current Primary',()=>{
 const dsh={...native,agent_id:'agent_dsh_default',settings:{state:'configured',current_selection:{mode:'dsh_additional',allowed_plan_ids:['primary'],plan_protocols:{primary:'responses'}}}};
 const plan={agent_plan_id:'primary',head:{status:'enabled'},desired:{mode:'fixed_model',strategy:{candidates:[{binding_id:'b'}]}}};
 assert.equal(currentPrimary(dsh,[plan]),undefined);
});

test('new aliases are bounded, readable and deterministic for the flow nonce',()=>{
 assert.equal(createTrialAlias('Example Flash!', '12ab34cd'),'trial-example-flash-12ab34cd');
 assert.equal(createTrialAlias('中文 !!', '12ab34cd'),'trial-model-12ab34cd');
 assert.equal(createTrialAlias('A'.repeat(100), '12ab34cd'),`trial-${'a'.repeat(40)}-12ab34cd`);
 assert.throws(()=>createTrialAlias('Model','bad'),/INVALID_TRIAL_NONCE/);
});
