import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../public/scripts/language.js', import.meta.url), 'utf8');
// Redirect pages as RedirectPage renders them: the root and a former Chinese URL.
const root = { redirect: '/en/', redirectZh: '/zh/' };
const former = pathname => ({ redirect: `/zh${pathname}` });
class Element {
  constructor() { this.attributes = {}; this.children = []; this.listeners = {}; }
  setAttribute(name, value) { this.attributes[name] = value; }
  getAttribute(name) { return this.attributes[name]; }
  addEventListener(type, handler) { this.listeners[type] = handler; }
  append(...children) { this.children.push(...children); }
  prepend(child) { this.children.unshift(child); }
  remove() { this.removed = true; }
}
function visit(path, languages, stored, blocked = false, dataset = {}) {
  const navigation = [];
  const listeners = {};
  const storage = new Map(stored ? [['hiroute-language', stored]] : []);
  const { pathname, hash } = new URL('https://hiroute.ai' + path);
  const english = pathname.startsWith('/en/');
  // The header language switch links each page to its counterpart.
  const counterpart = pathname.replace(/^\/(zh|en)\//, '/');
  const links = Object.fromEntries(Object.entries({ 'zh-CN': '/zh' + counterpart, en: '/en' + counterpart }).map(([lang, href]) => {
    const link = Object.assign(new Element(), { lang });
    link.setAttribute('href', href);
    return [lang, link];
  }));
  const body = new Element();
  vm.runInNewContext(source, {
    URL,
    location: { href: 'https://hiroute.ai' + path, hash, replace: value => navigation.push(['replace', value]) },
    navigator: { languages, language: languages[0] },
    localStorage: {
      getItem: key => { if (blocked) throw Error('blocked'); return storage.get(key); },
      setItem: (key, value) => { if (blocked) throw Error('blocked'); storage.set(key, value); },
    },
    document: {
      documentElement: { lang: english ? 'en' : 'zh-CN', dataset },
      body,
      addEventListener: (type, handler) => { listeners[type] = handler; },
      querySelector: selector => links[selector.match(/\[lang="([^"]+)"\]/)?.[1]] ?? null,
      createElement: () => new Element(),
    },
  });
  listeners.DOMContentLoaded?.();
  const banner = body.children[0];
  const [message, link, dismiss] = banner?.children[0].children ?? [];
  const redirected = navigation.find(([action]) => action === 'replace')?.[1];
  return { navigation, redirected, storage, click: listeners.click, banner, message, link, dismiss };
}

test('the root opens English for every browser language unless Chinese was chosen', () => {
  for (const languages of [['zh-CN'], ['zh-TW'], ['en-US', 'zh-CN'], ['fr-FR']]) {
    const page = visit('/', languages, undefined, false, root);
    assert.deepEqual(page.navigation, [['replace', 'https://hiroute.ai/en/']]);
    assert.equal(page.click, undefined);
  }
  assert.equal(visit('/', ['en'], 'zh', false, root).redirected, 'https://hiroute.ai/zh/');
  assert.equal(visit('/?lang=zh#tour', ['en'], 'en', false, root).redirected, 'https://hiroute.ai/zh/?lang=zh#tour');
  assert.equal(visit('/?lang=en#how', ['zh'], 'zh', false, root).redirected, 'https://hiroute.ai/en/?lang=en#how');
  assert.equal(visit('/?lang=zh', ['en'], undefined, true, root).redirected, 'https://hiroute.ai/zh/?lang=zh');
  const saved = visit('/?lang=zh', ['en'], undefined, false, root);
  assert.equal(saved.storage.get('hiroute-language'), 'zh');
});

test('former Chinese URLs open the Chinese page with their query and fragment', () => {
  for (const stored of [undefined, 'zh', 'en']) {
    const page = visit('/docs/models/?ref=readme#subscriptions', ['en-US'], stored, false, former('/docs/models/'));
    assert.deepEqual(page.navigation, [['replace', 'https://hiroute.ai/zh/docs/models/?ref=readme#subscriptions']]);
  }
  assert.equal(visit('/docs/', ['zh-CN'], undefined, true, former('/docs/')).redirected, 'https://hiroute.ai/zh/docs/');
});

test('browser language suggests the other version without redirecting', () => {
  for (const languages of [['en-US', 'zh-CN'], ['fr-FR']]) {
    const page = visit('/zh/?campaign=test#how', languages);
    assert.equal(page.redirected, undefined);
    assert.equal(page.banner.lang, 'en');
    assert.equal(page.message.textContent, 'This page is also available in English.');
    assert.equal(page.link.href, '/en/');
    assert.equal(page.link.lang, 'en');
    assert.equal(page.link.getAttribute('data-language'), '');
    assert.equal(page.dismiss.getAttribute('aria-label'), 'Dismiss');
  }
  for (const language of ['zh-CN', 'zh-TW', 'zh-HK', 'zh']) {
    const page = visit('/en/', [language]);
    assert.equal(page.redirected, undefined);
    assert.equal(page.message.textContent, '本页也有中文版。');
    assert.equal(page.link.href, '/zh/');
    assert.equal(visit('/zh/', [language]).banner, undefined);
  }
});

test('saved and explicit language choices keep direct pages without a suggestion', () => {
  for (const path of ['/en/', '/zh/', '/zh/docs/cli/', '/en/docs/cli/']) {
    for (const stored of ['zh', 'en']) {
      const page = visit(path, [stored === 'zh' ? 'en' : 'zh'], stored);
      assert.deepEqual(page.navigation, [], `${path} stays put`);
      assert.equal(page.banner, undefined, `${path} keeps a saved choice without a suggestion`);
    }
  }
  const explicit = visit('/zh/?lang=zh', ['en'], 'en', true);
  assert.deepEqual(explicit.navigation, []);
  assert.equal(explicit.banner, undefined);
});

test('direct pages suggest their counterpart, and dismissal keeps the current language', () => {
  for (const blocked of [false, true]) {
    const page = visit('/en/docs/cli/', ['zh-CN'], undefined, blocked);
    assert.equal(page.banner.lang, 'zh-CN');
    assert.equal(page.message.textContent, '本页也有中文版。');
    assert.equal(page.link.href, '/zh/docs/cli/');
    assert.equal(page.dismiss.getAttribute('aria-label'), '关闭');
    page.dismiss.listeners.click();
    assert.equal(page.banner.removed, true);
    assert.equal(page.storage.get('hiroute-language'), blocked ? undefined : 'en');
  }
  assert.equal(visit('/zh/docs/cli/', ['en-GB']).link.href, '/en/docs/cli/');
  assert.equal(visit('/en/docs/cli/', ['en-US']).banner, undefined);
});

test('manual language switch preserves the section and works when storage is unavailable', () => {
  for (const blocked of [false, true]) {
    const page = visit('/en/#how', ['en'], undefined, blocked);
    const link = { href: 'https://hiroute.ai/zh/', lang: 'zh-CN' };
    page.click({ target: { closest: () => link } });
    assert.equal(link.href, 'https://hiroute.ai/zh/?lang=zh#how');
    if (!blocked) assert.equal(page.storage.get('hiroute-language'), 'zh');
  }
});
