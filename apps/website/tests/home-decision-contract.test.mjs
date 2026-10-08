import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const homepage = fs.readFileSync(new URL('../src/components/HomePage.astro', import.meta.url), 'utf8');
const interactions = fs.readFileSync(new URL('../public/scripts/home.js', import.meta.url), 'utf8');

test('homepage extension response uses the current canonical first-turn decision contract', () => {
  const example = homepage.match(/<div class="code-box">[\s\S]*?<pre><code>([\s\S]*?)<\/code><\/pre>/)?.[1];
  assert(example, 'The extension response example must be present');
  const visible = example.replace(/<[^>]+>/g, '').replaceAll('&#123;', '{').replaceAll('&#125;', '}');
  const response = JSON.parse(visible.slice(visible.indexOf('{')));
  const canonical = JSON.parse(fs.readFileSync(new URL('../../../decision-extensions/api/decision-examples.json', import.meta.url)));
  assert.deepEqual(response, canonical.cases.find(item => item.name === 'smart_saving').response);
  assert.equal('assessment' in response, false, 'A first decision with no assessment target cannot report a score');
});

function visit(language) {
  const translate = (zh, en) => language === 'en' ? en : zh;
  const fields = Object.fromEntries(['kicker', 'title', 'copy', 'assessment', 'choice'].map(field => {
    const expression = homepage.match(new RegExp(`id="boundary-${field}"[^>]*>\\{(t\\([\\s\\S]*?\\))\\}`))?.[1];
    assert(expression, `Missing initial boundary field: ${field}`);
    return [`boundary-${field}`, { textContent: vm.runInNewContext(expression, { t: translate }) }];
  }));
  const buttons = [...homepage.matchAll(/data-boundary="([^"]+)" aria-pressed="([^"]+)"/g)].map(match => ({
    dataset: { boundary: match[1] },
    pressed: match[2],
    addEventListener(_, handler) { this.click = handler; },
    setAttribute(name, value) { assert.equal(name, 'aria-pressed'); this.pressed = value; },
  }));
  vm.runInNewContext(interactions, {
    URL,
    HTMLDetailsElement: class {},
    location: { hash: '' },
    window: { addEventListener() {} },
    document: {
      documentElement: { lang: language },
      querySelectorAll(selector) { return selector === '[data-boundary]' ? buttons : []; },
      getElementById(id) { return fields[id]; },
    },
  });
  return { buttons, values: () => Object.fromEntries(Object.entries(fields).map(([id, field]) => [id, field.textContent])) };
}

for (const language of ['zh-CN', 'en']) {
  test(`homepage routing boundaries stay aligned before and after interaction (${language})`, () => {
    const page = visit(language);
    assert.deepEqual(page.buttons.map(button => button.dataset.boundary), ['continuation', 'human']);
    const initial = page.values();
    page.buttons[0].click();
    assert.deepEqual(page.values(), initial, 'Clicking the initially selected continuation must not reveal stale routing copy');
    assert.match(initial['boundary-title'], /tool returns|工具返回/);
    page.buttons[1].click();
    const fresh = page.values();
    assert.match(fresh['boundary-title'], /new user turn|新用户轮次/);
    assert.deepEqual(page.buttons.map(button => button.pressed), ['false', 'true']);
    for (const id of Object.keys(initial)) {
      assert.notEqual(fresh[id], initial[id], `The new-turn explanation must update ${id}`);
    }
  });
}
