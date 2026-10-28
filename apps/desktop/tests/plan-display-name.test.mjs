import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { planDisplayNameProblem, planDisplayNameError } from '../src/plan-display-name.ts';

test('frontend accepts exactly the shared domain name examples', async () => {
  const cases = JSON.parse(await readFile(new URL('../../../contracts/routing/plan-display-name-conformance.v1.json', import.meta.url), 'utf8'));
  for (const row of cases) {
    const value = row.parts.map(part => typeof part === 'string' ? part : part.repeat.repeat(part.count)).join('');
    assert.equal(planDisplayNameProblem(value) === null, row.valid, row.case);
    for (const language of ['zh', 'en']) {
      const error = planDisplayNameError(value, language);
      assert.equal(error === null, row.valid, `${row.case} ${language}`);
      if (error && value.length >= 10) assert.ok(!error.includes(value), 'error must not repeat the input');
    }
  }
  assert.ok(planDisplayNameProblem('\ud800'), 'unpaired UTF-16 cannot reach the Rust string domain');
});
