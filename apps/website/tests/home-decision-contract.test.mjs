import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';

const homepage = fs.readFileSync(new URL('../src/components/HomePage.astro', import.meta.url), 'utf8');

test('homepage extension response uses the current canonical first-turn decision contract', () => {
  const example = homepage.match(/<div class="code-box">[\s\S]*?<pre><code>([\s\S]*?)<\/code><\/pre>/)?.[1];
  assert(example, 'The extension response example must be present');
  const visible = example.replace(/<[^>]+>/g, '').replaceAll('&#123;', '{').replaceAll('&#125;', '}');
  const response = JSON.parse(visible.slice(visible.indexOf('{')));
  const canonical = JSON.parse(fs.readFileSync(new URL('../../../decision-extensions/api/decision-examples.json', import.meta.url)));
  assert.deepEqual(response, canonical.cases.find(item => item.name === 'smart_saving').response);
  assert.equal('assessment' in response, false, 'A first decision with no assessment target cannot report a score');
});
