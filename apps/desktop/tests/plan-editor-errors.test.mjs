import { test } from 'node:test';
import assert from 'node:assert/strict';
import { planErrorCode, planErrorMessage } from '../src/plan-editor-errors.ts';
test('backend envelope is decoded without exposing payloads or stack traces', () => {
 const failure = {source:'backend', envelope:{error:{code:'INVALID_ARGUMENTS',details:'private-debug-content'}}};
 assert.equal(planErrorCode(failure),'INVALID_ARGUMENTS');
 assert.match(planErrorMessage(failure,'zh'),/校验/);
 assert.ok(!planErrorMessage(failure,'zh').includes('private-debug-content'));
});
test('all live consumers and both capacity dimensions are shown without native payloads', () => {
 const blocker = (kind, context) => ({ agent_kind: kind, context_id: `agent-context/${context}/sha256:abc`, plan_id: 'plan/free', model_alias: 'free.plan', reason: 'capacity_reduced', declared: { context_window_tokens: 262144, max_output_tokens: 235929 }, proposed: { context_window_tokens: 262144, max_output_tokens: 131072 } });
 const conflict = { schema: 'hiroute.agent-model-budget-conflict/v1', blockers: [blocker('pi', 'pi'), blocker('deepseek_harness', 'dsh'), blocker('opencode', 'opencode')] };
 const error = { envelope: { error: { code: 'REVISION_CONFLICT', details: 'private-native-config', model_budget_conflict: conflict } } };
 assert.equal(planErrorCode(error), 'AGENT_MODEL_BUDGET_CONFLICT');
 const message = planErrorMessage(error, 'zh');
 assert.match(message, /Pi「free.plan」.*最大输出 235,929 → 131,072.*DeepSeek Harness「free.plan」.*OpenCode「free.plan」/);
 assert.doesNotMatch(message, /Qoder|private-native-config|上下文 262/);
 assert.match(message, /取消接入受影响计划.*任务协作无需停用.*输入已保留/);
 assert.match(planErrorMessage({ safe_error_code: 'AGENT_MODEL_BUDGET_CONFLICT', model_budget_conflict: conflict }, 'en'), /Pi.*max output.*DeepSeek Harness.*OpenCode/);
 const contextOnly = { ...conflict, blockers: [{ ...blocker('qoder', 'qoder'), proposed: { context_window_tokens: 32000, max_output_tokens: 235929 } }] };
 assert.match(planErrorMessage({ code: 'QODER_MODEL_BUDGET_CONFLICT', model_budget_conflict: contextOnly }, 'zh'), /Qoder.*上下文 262,144 → 32,000/);
 for (const reason of ['plan_missing', 'budget_unavailable']) {
  const value = { ...conflict, blockers: [{ ...blocker('pi', 'pi'), reason, proposed: null }] };
  assert.match(planErrorMessage({ code: 'AGENT_MODEL_BUDGET_CONFLICT', model_budget_conflict: value }, 'zh'), reason === 'plan_missing' ? /计划不可用/ : /预算不可计算/);
 }
});
test('malformed capacity projections fall back safely', () => {
 for (const blockers of [[], [null], [{ agent_kind: 'qoder', context_id: '/private/settings.json' }]]) {
  const error = { envelope: { error: { code: 'REVISION_CONFLICT', model_budget_conflict: { schema: 'hiroute.agent-model-budget-conflict/v1', blockers } } } };
  assert.equal(planErrorCode(error), 'REVISION_CONFLICT');
  assert.doesNotMatch(planErrorMessage(error, 'zh'), /settings.json|停用 Qoder/);
 }
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


test('protocol errors identify only the validated safe scope or field', () => {
  const error = { envelope: { error: { message_key:'native.protocol.state_reference_invalid' }, data: {schema:'hiroute.native-protocol-diagnostic/v1',scope:'branch/code',message:'untrusted-private-detail'} } };
  assert.match(planErrorMessage(error, 'en'), /branch\/code/);
  assert.ok(!planErrorMessage(error, 'en').includes('untrusted-private-detail'));
  error.envelope.data.scope = '<private-wire-text>';
  assert.ok(!planErrorMessage(error, 'en').includes('<private-wire-text>'));
});
