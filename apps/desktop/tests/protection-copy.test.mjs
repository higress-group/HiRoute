import { test } from 'node:test';
import assert from 'node:assert/strict';
import { protectionExplanation } from '../src/features/trust/protection-copy.ts';

const classifier = connection => ({ kind: 'decision_service', service: { id: 'decision/test', revision: 1, name: 'Saved decision', connection } });
const builtin = classifier({ kind: 'system_one', provider: 'compatible', model: 'fixture', endpoint: 'https://decision.example.test', timeout_ms: 1000, auth_header: { name: 'Authorization', value_secret_ref: 'fixture' } });
const localRules = { kind: 'local_rules' };
const extension = classifier({ kind: 'custom', endpoint: 'https://decision.example.test', timeout_ms: 1000 });

for (const language of ['en', 'zh']) {
  test(`built-in protection keeps all three historical-applicability outcomes (${language})`, () => {
    const copy = protectionExplanation(builtin, language, 'Selected Primary');
    for (const phrase of language === 'zh' ? ['问题仍相关', '无法判断', '问题已修正或无关'] : ['earlier problem still matters', 'cannot be checked', 'fixed or unrelated']) assert.ok(copy.includes(phrase), copy);
    assert.ok(copy.includes('Selected Primary'));
    for (const phrase of language === 'zh' ? ['上一实际阶段', '按难度选择模型', '不会出现第三组'] : ['previous actual stage', 'based on difficulty', 'no third group']) assert.ok(copy.includes(phrase), copy);
  });
  test(`local rules do not claim to assess or protect a preceding stage (${language})`, () => {
    const copy = protectionExplanation(localRules, language);
    for (const phrase of language === 'zh' ? ['本地规则', '不评估上一阶段', '不应用低分保护'] : ['Local rules', 'do not assess the previous stage', 'apply low-score protection']) assert.ok(copy.includes(phrase), copy);
  });
  test(`custom extension explains group selection and observational scores (${language})`, () => {
    const copy = protectionExplanation(extension, language);
    for (const phrase of language === 'zh' ? ['扩展', '选择', '可选评分', '观察', '不应用阈值或低分保护'] : ['extension chooses', 'Optional scores', 'observation only', 'no threshold or low-score protection']) assert.ok(copy.includes(phrase), copy);
    assert.doesNotMatch(copy, /next (?:agent )?turn goes to|下一轮交给|competence floor|胜任下限/);
  });
}
