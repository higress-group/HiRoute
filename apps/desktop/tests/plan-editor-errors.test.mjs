import { test } from 'node:test';
import assert from 'node:assert/strict';
import { planErrorCode, planErrorMessage } from '../src/plan-editor-errors.ts';
test('backend envelope is decoded without exposing payloads or stack traces', () => {
 const failure = {source:'backend', envelope:{error:{code:'INVALID_ARGUMENTS',details:'private-debug-content'}}};
 assert.equal(planErrorCode(failure),'INVALID_ARGUMENTS');
 assert.match(planErrorMessage(failure,'zh'),/校验/);
 assert.ok(!planErrorMessage(failure,'zh').includes('private-debug-content'));
});
test('unchanged publication is distinguishable from malformed failures', () => {
 assert.equal(planErrorCode({envelope:{error:{code:'PLAN_UNCHANGED'}}}),'PLAN_UNCHANGED');
 for(const value of [null, {}, new Error('private stack'), '{"code":"PLAN_UNCHANGED"}']) assert.equal(planErrorCode(value),'REQUEST_FAILED');
});
test('stale route and draft revisions have actionable messages', () => {
 assert.match(planErrorMessage('PLAN_HEAD_STALE', 'zh'), /生效配置/);
 assert.match(planErrorMessage('DRAFT_REVISION_STALE', 'en'), /latest draft/);
});
test('a Qoder budget conflict requires model reconnection and preserves independent task collaboration', () => {
 const failure = { source: 'backend', envelope: { error: {
  code: 'QODER_MODEL_BUDGET_CONFLICT', message_key: 'agent.error.qoder_model_budget_conflict', details: 'private-native-config',
 } } };
 assert.equal(planErrorCode(failure), 'QODER_MODEL_BUDGET_CONFLICT');
 assert.match(planErrorMessage(failure, 'zh'), /停用 Qoder 的模型路由.*再发布此计划并重新配置模型路由/);
 assert.match(planErrorMessage(failure, 'zh'), /任务协作无需停用/);
 assert.match(planErrorMessage(failure, 'en'), /Disable Qoder model routing.*publish this plan.*configure model routing again/);
 assert.ok(!planErrorMessage(failure, 'zh').includes('private-native-config'));
});
