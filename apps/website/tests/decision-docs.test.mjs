import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { renderDecisionDoc } from '../src/lib/decision-docs.mjs';

test('canonical decision documents render into bilingual website routes', async () => {
  const zh = await renderDecisionDoc('mechanism', 'zh');
  const en = await renderDecisionDoc('api', 'en');
  const jevZh = await renderDecisionDoc('jev', 'zh');
  const jevEn = await renderDecisionDoc('jev', 'en');
  const rendered = await Promise.all(['mechanism', 'api', 'jev'].flatMap(page =>
    ['zh', 'en'].map(language => renderDecisionDoc(page, language))));
  assert.match(zh, /href="\/docs\/jev-decider\/"/);
  assert.match(zh, /src="\/decision-assets\/quality-zh-CN\.png"/);
  assert.match(en, /href="\/api\/decision\.openapi\.json"/);
  assert.match(en, /href="\/en\/docs\/model-routing\/"/);
  assert.match(jevZh, /href="\/api\/jev-policy\.default\.json"/);
  assert.match(jevEn, /href="\/api\/jev-policy\.default\.json"/);
  assert.match(zh, /href="#illustration-evidence-boundaries"/);
  assert.match(zh, /id="illustration-evidence-boundaries"/);
  assert.match(rendered.find(html => html.includes('Illustration evidence boundaries')), /id="illustration-evidence-boundaries"/);
  assert.doesNotMatch(rendered.join(''), /href="[^" ]+\.md"/);
});

test('prepared OpenAPI is byte-identical to the one repository contract', () => {
  const canonical = fs.readFileSync(new URL('../../../decision-extensions/api/decision.openapi.json', import.meta.url));
  const prepared = fs.readFileSync(new URL('../public/api/decision.openapi.json', import.meta.url));
  assert.deepEqual(prepared, canonical);
  const parsed = JSON.parse(prepared);
  assert(parsed.paths['/v1/decisions']);
});

test('prepared Jev policy is byte-identical to the bundled default', () => {
  const canonical = fs.readFileSync(new URL('../../../decision-extensions/extensions/jev-decider/jev_decider/policy.default.json', import.meta.url));
  const prepared = fs.readFileSync(new URL('../public/api/jev-policy.default.json', import.meta.url));
  assert.deepEqual(prepared, canonical);
});
