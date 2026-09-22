import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../public/scripts/language.js', import.meta.url), 'utf8');
function visit(path, languages, stored, blocked = false) {
  let redirected;
  let click;
  const storage = new Map(stored ? [['hiroute-language', stored]] : []);
  vm.runInNewContext(source, {
    URL,
    location: { href: 'https://hiroute.ai' + path, hash: new URL('https://hiroute.ai' + path).hash,
      replace: value => { redirected = value; } },
    navigator: { languages, language: languages[0] },
    localStorage: {
      getItem: key => { if (blocked) throw Error('blocked'); return storage.get(key); },
      setItem: (key, value) => { if (blocked) throw Error('blocked'); storage.set(key, value); },
    },
    document: { addEventListener: (_, handler) => { click = handler; } },
  });
  return { redirected, storage, click };
}

test('root follows the preferred browser language, retaining query and fragment', () => {
  assert.equal(visit('/?campaign=test#how', ['en-US', 'zh-CN']).redirected,
    'https://hiroute.ai/en/?campaign=test#how');
  for (const language of ['zh-CN', 'zh-TW', 'zh-HK', 'zh']) {
    assert.equal(visit('/', [language]).redirected, undefined);
  }
  assert.equal(visit('/', ['fr-FR']).redirected, 'https://hiroute.ai/en/');
});

test('explicit and saved language choices win, while direct routes remain stable', () => {
  assert.equal(visit('/', ['en'], 'zh').redirected, undefined);
  assert.equal(visit('/', ['zh'], 'en').redirected, 'https://hiroute.ai/en/');
  assert.equal(visit('/?lang=zh', ['en'], 'en', true).redirected, undefined);
  for (const path of ['/en/', '/docs/cli/', '/en/docs/cli/']) {
    assert.equal(visit(path, ['en'], 'zh').redirected, undefined);
  }
});

test('manual language switch preserves the section and works when storage is unavailable', () => {
  for (const blocked of [false, true]) {
    const page = visit('/en/#how', ['en'], undefined, blocked);
    const link = { href: 'https://hiroute.ai/', lang: 'zh-CN' };
    page.click({ target: { closest: () => link } });
    assert.equal(link.href, 'https://hiroute.ai/?lang=zh#how');
    if (!blocked) assert.equal(page.storage.get('hiroute-language'), 'zh');
  }
});
