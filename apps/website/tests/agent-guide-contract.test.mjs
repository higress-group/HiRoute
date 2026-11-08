import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import { validateAgentGuide, validateAgentGuidePage } from '../src/lib/agent-guide-contract.mjs';
import { renderUserGuide } from '../src/lib/user-guides.mjs';

test('a new guide cannot omit the copyable handoff or its useful references', async () => {
  const source = await fs.readFile(new URL('../content/guides/models.en.md', import.meta.url), 'utf8');
  const message = validateAgentGuide(source, 'models', 'en');
  assert.ok(message.includes('compute connection preview'));
  for (const mutate of [
    value => value.replace(/<aside[\s\S]*?<\/aside>/, ''),
    value => value + '\n<aside class="agent-guide" id="agent-cli"></aside>',
    value => value.replace('class="agent-guide"', 'class="agent-guide-other"'),
    value => value.replace('class="agent-guide-heading"', 'class="heading"'),
    value => value.replace('class="agent-guide-preview"', 'class="preview"'),
    value => value.replace('```text', '```sh'),
    value => value.replace('https://hiroute.ai/en/docs/models/', 'https://hiroute.ai/zh/docs/models/'),
    value => value.replace('https://hiroute.ai/en/docs/cli/', 'https://example.test/cli/'),
    value => value.replace('https://hiroute.ai/en/docs/#management-skill', 'https://hiroute.ai/en/docs/'),
    value => value.replace('--include-schema', '--invented'),
  ]) assert.throws(() => validateAgentGuide(mutate(source), 'models', 'en'));
});

test('delegation instructions retain the product Skill activation path', async () => {
  const source = await fs.readFile(new URL('../content/guides/task-routing.zh.md', import.meta.url), 'utf8');
  validateAgentGuide(source, 'task-routing', 'zh');
  assert.throws(() => validateAgentGuide(source.replace('https://hiroute.ai/zh/docs/task-routing/#delegation-skill',
    'https://hiroute.ai/zh/docs/task-routing/'), 'task-routing', 'zh'), /delegation-skill/);
});

test('a rendered guide cannot silently lose its copy interaction', async () => {
  const guide = await renderUserGuide('models', 'en');
  const page = `${guide}<script src="/scripts/guide-agent-copy.js" defer></script>`;
  validateAgentGuidePage(page, '/en/docs/models/');
  assert.throws(() => validateAgentGuidePage(guide, '/en/docs/models/'), /Copy for Agent/);
});
