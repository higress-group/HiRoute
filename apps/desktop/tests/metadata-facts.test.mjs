import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {capabilityText,inferenceRule,metadataReferences,tokenLimitText} from '../src/features/model-feed/metadata-facts.ts';

const data=JSON.parse(readFileSync(new URL('../../../assets/release-facts/current/bundle/official-model-data.json',import.meta.url))).data.metadata_catalog;
test('actual catalog inference stays distinct from observed capability evidence',()=>{
  const inferred=data.model_records.find(record=>record.model_record_key==='hermes-agent/ai-gateway/google%2Fgemini-3-flash');
  const observed=data.model_records.find(record=>record.model_record_key==='hermes-agent/openai-codex/gpt-5.3-codex-spark');
  assert.equal(inferenceRule(inferred,'capability_hints.tool'),'rule.google.gemini/v1');
  assert.equal(inferenceRule(inferred,'context_tokens'),'rule.google.gemini/v1');
  assert.equal(inferenceRule(observed,'capability_hints.tool'),null);
  assert.deepEqual(metadataReferences(inferred),[{kind:'client_run',key:'hermes-agent'},{kind:'inference_rule',key:'rule.google.gemini/v1'}]);
  assert.ok(metadataReferences(observed).some(ref=>ref.kind==='evidence_source'));
});
test('missing, conditional, conflicting and inapplicable facts do not become negative capability or zero limits',()=>{
  for(const zh of [false,true]) {
    const states=['supported','unsupported','unknown','conditional','conflict','runtime-required','not-applicable'];
    assert.equal(new Set(states.map(state=>capabilityText(state,zh))).size,states.length);
    const limits=['unknown','not-applicable','entitlement-dependent','conflict','shared-total-budget','runtime-required'].map(state=>({state}));
    assert.equal(new Set(limits.map(value=>tokenLimitText(value,zh))).size,limits.length);
    assert.equal(tokenLimitText({state:'known',value:0},zh),'0');
  }
});
