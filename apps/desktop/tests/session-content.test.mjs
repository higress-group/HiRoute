import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readableEvent, excerpt, excerptAround, groupToolEvents, summarizeToolArguments, contentDisplayRuns, coalesceResponseText, isPrivateContentKind, isTechnicalContentKind } from '../src/features/session-content.ts';
const media = 'application/vnd.hiroute.model-stream-event+json;version=1';
const encode = event => JSON.stringify({ schema_version: 'hiroute.model-stream-event/v1', sequence: 1, event });
test('canonical text is read only from the declared response media type', () => {
 const raw = encode({ kind: 'text_delta', index: 0, text: '<script>hello</script>' });
 assert.deepEqual(readableEvent(raw, media, 'response_delivered'), { kind: 'text', text: '<script>hello</script>', sequence: 1, index: 0, phase: 'delta' });
 assert.equal(readableEvent(raw, 'text/plain', 'response_delivered'), null);
 assert.equal(readableEvent(raw, media, 'request_input'), null);
});
test('technical context is grouped without moving or hiding user turns', () => {
 assert.deepEqual(contentDisplayRuns([{role:'system'},{role:'developer'},{role:'user'},{role:'tool_definition'},{role:'assistant'}]), [
  {start:0,end:2,technical:true}, {start:2,end:3,technical:false}, {start:3,end:4,technical:true}, {start:4,end:5,technical:false},
 ]);
 assert.deepEqual(contentDisplayRuns([{role:'user'},{role:'assistant'}]), [{start:0,end:2,technical:false}]);
 assert.deepEqual(contentDisplayRuns([]), []);
 assert.deepEqual(contentDisplayRuns([{role:'assistant',technical:true},{role:'assistant',technical:false}]), [{start:0,end:1,technical:true},{start:1,end:2,technical:false}]);
});
test('canonical kinds keep provider state and reasoning private while classifying tool fields', () => {
 assert.equal(isPrivateContentKind('provider_state'), true);
 assert.equal(isPrivateContentKind('reasoning_delta'), true);
 assert.equal(isPrivateContentKind('reasoning_finished'), true);
 assert.equal(isPrivateContentKind('text'), false);
 assert.equal(isTechnicalContentKind('tool_call_arguments'), true);
 assert.equal(isTechnicalContentKind('tool_result_text'), true);
 assert.equal(isTechnicalContentKind('message_name'), true);
 assert.equal(isTechnicalContentKind('text'), false);
});
test('delta fragments are contiguous and a finished snapshot replaces them once', () => {
 const parse = (sequence, kind, text, index = 0) => readableEvent(JSON.stringify({ schema_version: 'hiroute.model-stream-event/v1', sequence, event: { kind, index, text } }), media, 'response_delivered');
 const fragments = [parse(3, 'text_delta', 'R-'), parse(1, 'text_delta', 'HI'), parse(4, 'text_delta', 'OK'), parse(2, 'text_delta', '-')];
 assert.deepEqual(coalesceResponseText(fragments), [{ kind: 'text', index: 0, text: 'HI-R-OK' }]);
 assert.deepEqual(coalesceResponseText([...fragments, parse(5, 'text_finished', 'HIR-TOOL-OK-SEP21')]), [{ kind: 'text', index: 0, text: 'HIR-TOOL-OK-SEP21' }]);
 assert.deepEqual(coalesceResponseText([parse(5, 'text_finished', 'Only final')]), [{ kind: 'text', index: 0, text: 'Only final' }]);
 assert.deepEqual(coalesceResponseText([parse(1, 'reasoning_finished', 'private'), parse(2, 'refusal_finished', 'Cannot do that')]), [{ kind: 'refusal', index: 0, text: 'Cannot do that' }]);
});
test('tool identity keeps namespace and distinguishes arguments from execution results', () => {
 const start = readableEvent(encode({ kind: 'tool_call_started', logical_id: 'call/1', namespace: 'math', name: 'add' }), media, 'response_delivered');
 assert.equal(start.name, 'math.add'); assert.equal(start.phase, 'started');
 const finish = readableEvent(encode({ kind: 'tool_call_finished', logical_id: 'call/1', name: 'add', arguments: { n: 2 } }), media, 'response_delivered');
 assert.equal(finish.phase, 'ready'); assert.match(finish.arguments, /"n": 2/);
});
test('partial, unrecognized and wrong-version content remains raw', () => {
 for (const raw of ['{"schema_version":', encode({ kind: 'provider_state', state: {} }), encode({ kind: 'text_delta', text: 3 }), encode({ kind: 'text_delta', text: 'hi' }).replace('/v1', '/v2')]) assert.equal(readableEvent(raw, media, 'response_delivered'), null);
});
test('excerpt preserves multibyte text and surrogate pairs with bounded output', () => {
 assert.equal(excerpt('你好🙂世界', 3), '你好🙂…');
 assert.equal(excerpt(' a \n b '), 'a b');
});

test('tool grouping orders chunks, keeps calls separate and uses final arguments without claiming execution', () => {
 const tool = (sequence, logicalId, phase, name, args) => ({ kind: 'tool', sequence, logicalId, phase, name, arguments: args });
 const result = groupToolEvents([tool(4, 'a', 'ready', 'math.add', '{"n":2}'), tool(2, 'a', 'arguments', '', '{'), tool(1, 'a', 'started', 'math.add'), tool(3, 'b', 'started', 'math.add')]);
 assert.equal(result.length, 2); assert.equal(result[0].name, 'math.add'); assert.equal(result[0].phase, 'ready'); assert.equal(result[0].arguments, '{"n":2}'); assert.equal(result[1].phase, 'started');
 assert.equal(groupToolEvents([tool(2, 'a', 'arguments', '', 'b'), tool(1, 'a', 'arguments', '', 'a')])[0].arguments, 'ab');
});
test('tool argument summaries keep useful values compact without exposing transport syntax', () => {
 assert.equal(summarizeToolArguments('{"paths":["execution_plan.rs","driver.rs"]}'), 'execution_plan.rs, driver.rs');
 assert.equal(summarizeToolArguments('{"line":12,"enabled":true}'), 'line: 12 · enabled: true');
});
test('search excerpt centers on the original UTF-8 anchor', () => {
 const before = '前'.repeat(300);
 const result = excerptAround(before + '命中内容' + '后'.repeat(300), new TextEncoder().encode(before).length);
 assert.match(result, /命中内容/); assert.ok(result.length <= 162); assert.ok(!result.includes('�'));
});
