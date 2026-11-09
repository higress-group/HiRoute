import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { installPlatforms } from '../src/lib/install-platforms.mjs';
import { latestRelease } from '../src/lib/releases.mjs';

const script = fs.readFileSync(new URL('../public/scripts/install.js', import.meta.url), 'utf8');

// Exercise the shipped navigation handler with the same native fragment links
// as the static page. Browser acceptance covers layout, real keys and clipboard.
function visit(hash = '', viewportHeight = 900) {
  const listeners = {};
  const windowListeners = {};
  const history = [];
  const scrolls = [];
  const location = { hash };
  const prompt = { hidden: true };
  let focused;
  function link(id, platform) {
    const attributes = {};
    return {
      hash: `#${id}`, dataset: { installPlatform: platform },
      setAttribute: (name, value) => { attributes[name] = value; },
      removeAttribute: name => { delete attributes[name]; },
      getAttribute: name => attributes[name],
      closest() { return this; },
      focus() { focused = this; },
    };
  }
  const platforms = installPlatforms(latestRelease(), 'en');
  const cards = platforms.map(platform => link(platform.id, platform.id));
  const navigation = { querySelectorAll: () => cards, scrollIntoView: options => scrolls.push(options) };
  for (const card of cards) card.parentElement = navigation;
  const panels = platforms.map(platform => {
    const artifacts = platform.kind === 'desktop'
      ? platform.artifacts.map(artifact => ({ id: `${platform.id}-${artifact.architecture}` })) : [];
    const options = artifacts.map(artifact => link(artifact.id));
    for (const option of options) option.parentElement = { querySelectorAll: () => options };
    return {
      id: platform.id, artifacts, options,
      querySelector: () => ({ getBoundingClientRect: () => ({ bottom: 765 }) }),
      querySelectorAll: selector => selector === '[data-install-artifact]' ? artifacts : options,
    };
  });
  const picker = {
    querySelector: () => prompt,
    querySelectorAll: selector => ({ '[data-install-platform]': cards, '[data-install-panel]': panels }[selector] ?? []),
    addEventListener: (name, callback) => { listeners[name] = callback; },
  };
  vm.runInNewContext(script, {
    location,
    history: { pushState: (_state, _title, value) => { history.push(value); location.hash = value; } },
    document: { querySelector: () => picker, documentElement: { lang: 'en' } },
    window: { innerHeight: viewportHeight, addEventListener: (name, callback) => { windowListeners[name] = callback; } },
  });
  return {
    cards, panels, prompt, history, scrolls,
    visible: () => panels.filter(panel => !panel.hidden).map(panel => panel.id),
    click(target, modifiers = {}) {
      let prevented = false;
      listeners.click({ target, button: 0, ...modifiers, preventDefault: () => { prevented = true; } });
      return prevented;
    },
    navigate(value, event = 'popstate') { location.hash = value; windowListeners[event](); },
    key(target, key) { listeners.keydown({ target, key, preventDefault() {} }); return focused; },
  };
}

test('all four platform choices remain present when a release or package is unavailable', () => {
  for (const release of [undefined, { artifacts: [] }]) {
    const platforms = installPlatforms(release, 'en');
    assert.deepEqual(platforms.map(platform => platform.name), ['macOS', 'Omarchy', 'Windows', 'Linux Server']);
    assert.ok(platforms.every(platform => platform.artifacts.length === 0 && platform.architectures === ''));
  }
  const release = latestRelease();
  for (const platform of installPlatforms(release, 'en')) {
    assert.ok(platform.artifacts.length > 0);
    for (const artifact of platform.artifacts) {
      assert.equal(artifact.platform, platform.platform);
      assert.equal(artifact.kind, platform.kind);
      assert.ok(release.artifacts.includes(artifact), 'use the exact published artifact');
    }
  }
});

test('fresh and unknown fragments leave the four choices neutral', () => {
  for (const hash of ['', '#unknown']) {
    const page = visit(hash);
    assert.equal(page.prompt.hidden, false);
    assert.deepEqual(page.visible(), []);
    assert.ok(page.cards.every(card => card.getAttribute('aria-current') === undefined));
  }
});

test('platform and architecture links restore the matching state through browser history', () => {
  const page = visit();
  const omarchy = page.panels.find(panel => panel.id === 'omarchy');
  const arm = omarchy.options.find(option => option.hash.endsWith('aarch64'));
  page.click(page.cards[1]);
  page.click(arm);
  assert.deepEqual(page.history, ['#omarchy', '#omarchy-aarch64']);
  assert.deepEqual(page.visible(), ['omarchy']);
  assert.deepEqual(omarchy.artifacts.filter(artifact => !artifact.hidden).map(artifact => artifact.id), ['omarchy-aarch64']);
  assert.equal(arm.getAttribute('aria-current'), 'true');
  page.navigate('#omarchy');
  assert.deepEqual(omarchy.artifacts.filter(artifact => !artifact.hidden).map(artifact => artifact.id), ['omarchy-x86_64']);
  page.navigate('');
  assert.deepEqual(page.visible(), []);
  assert.equal(page.prompt.hidden, false);
  page.navigate('#macos-x86_64', 'hashchange');
  assert.deepEqual(page.visible(), ['macos']);
  assert.equal(page.cards[0].getAttribute('aria-current'), 'true');
});

test('selecting a platform reveals offscreen details on a short viewport', () => {
  const short = visit('', 640);
  short.click(short.cards[1]);
  assert.deepEqual(short.visible(), ['omarchy']);
  assert.equal(short.scrolls.length, 1);
  short.click(short.panels[1].options.find(option => option.hash.endsWith('aarch64')));
  assert.equal(short.scrolls.length, 1);

  const tall = visit();
  tall.click(tall.cards[1]);
  assert.equal(tall.scrolls.length, 0);
});

test('direct links select the correct architecture and preserve modified navigation', () => {
  const page = visit('#omarchy-aarch64');
  assert.deepEqual(page.visible(), ['omarchy']);
  const omarchy = page.panels[1];
  assert.deepEqual(omarchy.artifacts.filter(artifact => !artifact.hidden).map(artifact => artifact.id), ['omarchy-aarch64']);
  for (const modifiers of [{ ctrlKey: true }, { metaKey: true }, { shiftKey: true }, { altKey: true }, { button: 1 }]) {
    assert.equal(page.click(page.cards[0], modifiers), false);
  }
  assert.deepEqual(page.history, []);
});

test('arrow keys move focus without changing the selected environment', () => {
  const page = visit('#windows');
  assert.equal(page.key(page.cards[0], 'ArrowRight'), page.cards[1]);
  assert.equal(page.key(page.cards[0], 'ArrowLeft'), page.cards[3]);
  assert.equal(page.key(page.cards[1], 'Home'), page.cards[0]);
  assert.equal(page.key(page.cards[1], 'End'), page.cards[3]);
  assert.deepEqual(page.visible(), ['windows']);
  assert.deepEqual(page.history, []);
});
