import assert from 'node:assert/strict';
import test from 'node:test';
import { trialReadiness } from '../src/features/trial/readiness.ts';

const valid = {
  language:'en',trialId:'trial',trialUsable:true,trialReasoning:'selected',primaryId:'primary',primaryUsable:true,primaryReasoning:'selected',
  connectionOpen:false,decisionUsable:true,agentId:'agent',agentMatches:true,agentSeedKnown:true,claude:true,presets:1,
  name:'Example',threshold:900,custom:false,authoring:true,loading:false,readError:false,decisionError:false,agentError:false,mutable:true,authority:true,
};

test('each review predicate produces a localized, navigable blocker', () => {
  const cases = [
    [{trialId:''},'trial-model'], [{trialUsable:false},'trial-binding'], [{connectionOpen:true},'trial-connection-open'],
    [{trialReasoning:'choice_required'},'trial-reasoning'], [{trialReasoning:'invalid'},'trial-reasoning'],
    [{primaryId:''},'primary-binding'], [{primaryUsable:false},'primary-binding'], [{primaryId:'trial'},'primary-distinct'],
    [{primaryReasoning:'choice_required'},'primary-reasoning'], [{primaryReasoning:'invalid'},'primary-reasoning'],
    [{decisionUsable:false},'decision'], [{agentId:''},'agent'], [{agentMatches:false},'agent-current'],
    [{agentSeedKnown:false},'agent-seed'], [{presets:0},'presets'], [{name:' '},'name'],
    [{threshold:-1},'threshold'], [{threshold:1001},'threshold'], [{threshold:723.5},'threshold'],
    [{loading:true},'loading'], [{readError:true},'read'], [{decisionError:true},'decision-read'],
    [{agentError:true},'agent-read'], [{authoring:false},'authoring'], [{mutable:false},'authority'], [{authority:false},'authority'],
  ];
  for (const language of ['en','zh']) for (const [patch,code] of cases) {
    const result=trialReadiness({...valid,language,...patch});
    const blocker=result.blockers.find(value=>value.code===code);
    assert.ok(blocker,`${language} ${code}`);
    assert.ok(blocker.message && blocker.target && blocker.category,`${language} ${code} has a navigable explanation`);
    assert.equal(result.canAdvance,false,`${language} ${code} cannot advance`);
  }
});

test('compound field blockers update individually without becoming system failures', () => {
  const input={...valid,trialUsable:false,primaryReasoning:'choice_required'};
  const initial=trialReadiness(input);
  assert.deepEqual(initial.blockers.map(value=>value.code),['trial-binding','primary-reasoning']);
  assert.equal(initial.systemBlocked,false);
  assert.deepEqual(trialReadiness({...input,trialUsable:true}).blockers.map(value=>value.code),['primary-reasoning']);
  assert.equal(trialReadiness({...input,trialUsable:true,primaryReasoning:'selected'}).canAdvance,true);
});

test('custom extensions and non-Claude Agents do not inherit irrelevant requirements', () => {
  assert.equal(trialReadiness({...valid,custom:true,threshold:-1,claude:false,presets:0}).canAdvance,true);
  assert.equal(trialReadiness({...valid,threshold:0}).canAdvance,true);
  assert.equal(trialReadiness({...valid,threshold:723}).canAdvance,true);
  assert.equal(trialReadiness({...valid,threshold:1000}).canAdvance,true);
});
