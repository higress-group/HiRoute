import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { renderDecisionDoc } from '../src/lib/decision-docs.mjs';

test('canonical decision documents render into bilingual website routes', async () => {
  const zh = await renderDecisionDoc('mechanism', 'zh');
  const rendered = await Promise.all(['mechanism', 'api', 'jev'].flatMap(page =>
    ['zh', 'en'].map(language => renderDecisionDoc(page, language))));
  assert.match(zh, /href="\/docs\/jev-decider\/"/);
  for (const html of rendered.slice(0, 4)) {
    assert.match(html, /href="\/api\/decision\.openapi\.json"/);
    assert.match(html, /href="\/api\/decision-examples\.json"/);
  }
  for (const html of rendered.slice(2, 4)) assert.match(html, /assessment_target/);
  assert.doesNotMatch(rendered.join(''), /href="(?!https:\/\/)[^" ]+\.md(?:#[^"]*)?"/);
  assert.match(rendered[4], /href="\/docs\/decision-api\/"/);
  assert.match(rendered[5], /href="\/en\/docs\/decision-api\/"/);
});

test('prepared OpenAPI and examples retain their canonical bytes', () => {
  const canonical = fs.readFileSync(new URL('../../../decision-extensions/api/decision.openapi.json', import.meta.url));
  const prepared = fs.readFileSync(new URL('../public/api/decision.openapi.json', import.meta.url));
  assert.ok(prepared.equals(canonical), 'Prepared OpenAPI differs from the current canonical contract; run prepare:content');
  const parsed = JSON.parse(prepared);
  assert(parsed.paths['/v1/decisions']);
  const examples = fs.readFileSync(new URL('../public/api/decision-examples.json', import.meta.url));
  assert.ok(examples.equals(fs.readFileSync(new URL('../../../decision-extensions/api/decision-examples.json', import.meta.url))), 'Prepared examples differ from the canonical examples');
  assert.deepEqual(JSON.parse(examples).cases.map(example => example.request.decision.kind), ['ordinal', 'categorical', 'categorical', 'subset']);
  assert.equal(fs.existsSync(new URL('../public/api/jev-policy.default.json', import.meta.url)), false, 'Removed extension policy must not survive in the website download assets');
});

test('native homepage screenshots are distinct, correctly sized and copied unchanged', () => {
  const captures = ['en', 'zh-CN'].map(language => {
    const name = `quality-native-${language}.png`;
    const source = fs.readFileSync(new URL(`../../../decision-extensions/assets/${name}`, import.meta.url));
    const prepared = fs.readFileSync(new URL(`../public/decision-assets/${name}`, import.meta.url));
    assert.deepEqual(prepared, source);
    assert.equal(source.subarray(1, 4).toString(), 'PNG');
    assert.equal(source.readUInt32BE(16), 2400);
    assert.equal(source.readUInt32BE(20), 2100);
    return source;
  });
  assert.notDeepEqual(captures[0], captures[1]);
});

// Rendered prose examples must stay identical to the maintained protocol examples.
test('API guides and OpenAPI expose current examples without future tool selection', () => {
  const source = JSON.parse(fs.readFileSync(new URL('../../../decision-extensions/api/decision-examples.json', import.meta.url)));
  const current = source.cases.filter(c => c.scope !== 'future-docs-only');
  for (const filename of ['README.md', 'README.zh-CN.md']) {
    const markdown = fs.readFileSync(new URL(`../../../decision-extensions/api/${filename}`, import.meta.url), 'utf8');
    const blocks = [...markdown.matchAll(/```json\n([\s\S]*?)\n```/g)].map(match => JSON.parse(match[1]));
    assert.deepEqual(blocks, current.flatMap(c => [c.request, c.response]));
  }
  const openapi = JSON.parse(fs.readFileSync(new URL('../public/api/decision.openapi.json', import.meta.url)));
  const examples = openapi.paths['/v1/decisions'].post.requestBody.content['application/json'].examples;
  assert.deepEqual(Object.keys(examples), current.map(c => c.name));
  assert.equal(examples.single_group_category.value.decision.options[1].refinement, undefined);
});
