import { scenario } from './scenario-selection.mjs';

const control = () => window.productShell;
const assert = (value, message) => { if (!value) throw new Error(message); };
const visible = element => element?.getClientRects().length && getComputedStyle(element).visibility === 'visible';
const all = selector => [...document.querySelectorAll(selector)].filter(visible);
const pause = () => new Promise(resolve => setTimeout(resolve, 25));
async function until(check, message) {
  const deadline = Date.now() + 6000;
  while (!check()) { if (Date.now() > deadline) throw new Error(message); await pause(); }
}
async function ready(language, theme) {
  const previousRoot = document.querySelector('.occami-root');
  control().reset();
  localStorage.setItem('hiroute.language', language);
  localStorage.setItem('hiroute.theme', theme);
  control().remount();
  await until(() => document.querySelector('.occami-root') !== previousRoot
    && visible(document.querySelector('.sidebar-health .status-dot.good'))
    && document.querySelector('.occami-root')?.dataset.theme === theme
    && all('button').some(button => button.getAttribute('aria-label') === (language === 'zh' ? '首页' : 'Home')), 'Presentation did not initialize');
}
async function navigate(label) {
  const button = all('button').find(button => button.getAttribute('aria-label') === label);
  assert(button, `Missing navigation: ${label}`); button.click(); await pause();
}
function bounded(element, label) {
  const bounds = element.getBoundingClientRect();
  assert(bounds.width > 0 && bounds.left >= -1 && bounds.right <= innerWidth + 1,
    `${label} is outside the viewport: ${bounds.left}..${bounds.right} of ${innerWidth}`);
}
function styledButtons(label) {
  for (const button of all('button')) {
    assert(getComputedStyle(button).borderTopStyle !== 'outset', `${label}: browser-default button ${button.textContent.trim()}`);
    bounded(button, `${label}: ${button.textContent.trim().slice(0, 50)}`);
  }
}

export const presentationScenarios = [
  scenario('desktop.settings.storage-capacity', ['presentation', 'storage'], 'Capacity save, disable and failure preserve the effective policy and usable local-record controls', async () => {
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) {
      await ready(language, theme);
      let effective = { max_active_bytes: 20_000_000_000, active_bytes: 18_000_000_000, measured_at_ms: 1000, state: 'idle', reason: 'within_limit' };
      control().handlers.observation_capacity_get = () => structuredClone(effective);
      control().handlers.observation_capacity_set = payload => { effective = { ...effective, ...payload.input }; return structuredClone(effective); };
      await navigate(language === 'zh' ? '设置' : 'Settings');
      all('[role="tab"]').find(button => button.textContent === (language === 'zh' ? '记录与诊断' : 'Records & diagnostics')).click(); await pause();
      const field = () => document.querySelector('input[aria-label="' + (language === 'zh' ? '记录容量上限（GB）' : 'Record storage limit (GB)') + '"]');
      await until(() => field() && !field().disabled, 'Capacity read did not finish');
      const set = value => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(field(), value); field().dispatchEvent(new Event('input', { bubbles: true })); };
      const save = () => field().closest('.settings-row').querySelector('button');
      assert(field().value === '20' && save().disabled, 'Default capacity does not show 20 GB');
      set('25'); await pause(); save().click();
      await until(() => effective.max_active_bytes === 25_000_000_000 && !field().disabled, 'Custom capacity did not save as decimal bytes');
      assert(control().commands.some(c => c.command === 'observation_capacity_set' && c.payload.input.max_active_bytes === 25_000_000_000), 'Save bypassed the shared capacity boundary');
      set('0'); await pause(); assert(save().disabled, 'Zero capacity can be saved');
      set('21'); await pause(); control().handlers.observation_capacity_set = () => { throw new Error('STORAGE_UNAVAILABLE'); }; save().click();
      await until(() => !field().disabled && field().closest('.settings-list').querySelector('.settings-inline-error')?.textContent.includes('STORAGE_UNAVAILABLE'), 'Failed save did not expose retry');
      assert(effective.max_active_bytes === 25_000_000_000 && field().value === '21', 'Failed save changed the effective policy or discarded the draft');
      control().handlers.observation_capacity_set = payload => { effective = { ...effective, ...payload.input }; return structuredClone(effective); };
      set(''); await pause(); save().click();
      await until(() => effective.max_active_bytes === null && !field().disabled, 'Empty capacity did not disable early expiry');
      control().remount(); await pause();
      await navigate(language === 'zh' ? '设置' : 'Settings');
      all('[role="tab"]').find(button => button.textContent === (language === 'zh' ? '记录与诊断' : 'Records & diagnostics')).click(); await pause();
      await until(() => field() && !field().disabled && field().value === '', 'Disabled policy was not retained when reopening settings');
      field().scrollIntoView({ block: 'center' }); await pause();
      const row = field().closest('.settings-row');
      assert(row.scrollWidth <= row.clientWidth + 1, 'Capacity row overflows');
      bounded(field(), 'Capacity input'); bounded(save(), 'Capacity save');
    }
  }),
  scenario('desktop.models.view-switch', ['model-connections', 'presentation'], 'Model/connection grouping has visible selection, usable controls and stable selected models', async () => {
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) {
      await ready(language, theme); await navigate(language === 'zh' ? '模型' : 'Models');
      const groupLabel = language === 'zh' ? '查看方式' : 'View';
      const group = () => all('[role="group"]').find(element => element.getAttribute('aria-label') === groupLabel);
      await until(() => group() && all('.models-feature .master-list > .list-row').length, 'Missing model view');
      const buttons = [...group().querySelectorAll('button')];
      assert(buttons.length === 2, 'Expected the two grouping choices');
      for (const button of buttons) {
        bounded(button, 'Grouping control');
        assert(button.getBoundingClientRect().height >= 28 && parseFloat(getComputedStyle(button).borderRadius) >= 6, 'Grouping control fell back to browser presentation');
      }
      const sources = control().management.sources;
      const modelCount = sources.reduce((count, source) => count + Math.max(1, source.models.length), 0);
      assert(modelCount > sources.length, 'Fixture must distinguish model and connection grouping');
      assert(all('.models-feature .master-list > .list-row').length === modelCount, 'Model grouping omitted saved models');
      const selected = all('.models-feature .master-list > .list-row')[1];
      const selectedName = selected.querySelector('.row-title').textContent;
      selected.click(); await pause();
      // Small windows show one pane at a time; return through the real control.
      const back = all('button').find(button => button.textContent.trim() === (language === 'zh' ? '返回模型列表' : 'Back to models'));
      if (back && !visible(group())) { back.click(); await pause(); }
      buttons[1].click();
      await until(() => all('.models-feature .master-list > .list-row').length === sources.length, 'Connections were not grouped once per source');
      assert(buttons[1].getAttribute('aria-pressed') === 'true' && buttons[0].getAttribute('aria-pressed') === 'false', 'Grouping selection is ambiguous');
      assert(getComputedStyle(buttons[0]).backgroundColor !== getComputedStyle(buttons[1]).backgroundColor, 'Selected grouping has no visible state');
      buttons[0].click(); await pause();
      assert(all('.models-feature .master-list > .list-row').length === modelCount, 'Returning to models lost entries');
      assert(all('.models-feature .list-row[aria-current="page"]')[0]?.textContent.includes(selectedName), 'Grouping discarded the selected model');
      styledButtons('Models');
    }
  }),
  scenario('desktop.presentation.control-layout', ['presentation'], 'Main pages retain styled controls and long recent sessions fit the supported small window', async () => {
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) {
      await ready(language, theme);
      const pages = language === 'zh' ? ['首页', '模型', '智能路由', 'Agent', '会话', '设置']
        : ['Home', 'Models', 'Smart routing', 'Agent', 'Sessions', 'Settings'];
      for (const page of pages) {
        await navigate(page);
        if (page === '首页' || page === 'Home') {
          await until(() => all('.v3-recent').length > 0, 'Missing actual recent-session rows');
          await until(() => all('.v3-recent .row-title').some(title => title.textContent.length >= 40), 'Long recent-session content did not load');
          const content = all('.v3-home')[0].closest('.page-content');
          assert(content.scrollWidth <= content.clientWidth + 1, 'Recent sessions force horizontal scrolling');
        }
        styledButtons(page);
      }
    }
  }),
];
