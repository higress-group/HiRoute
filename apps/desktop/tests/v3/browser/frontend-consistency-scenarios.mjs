import { scenario } from './scenario-selection.mjs';
import { consistencySettingsReads } from './frontend-consistency-fixtures.mjs';

const control = () => window.productShell;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const visible = node => Boolean(node?.getClientRects().length);
const all = selector => [...document.querySelectorAll(selector)].filter(visible);
const assert = (condition, message) => { if (!condition) throw new Error(message); };
async function until(check, label) {
  for (let i = 0; i < 240; i++) { if (check()) { await pause(40); return; } await pause(25); }
  throw new Error(`Timed out: ${label}`);
}
async function click(label) {
  const target = () => all('button').find(node => node.textContent.trim() === label);
  await until(target, label); target().click(); await pause(60);
}
function input(node, value) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(node, value);
  node.dispatchEvent(new Event('input', { bubbles: true }));
}
async function fresh(language = 'zh') {
  control().reset();
  consistencySettingsReads(control());
  localStorage.setItem('hiroute.language', language); control().remount();
  await until(() => visible(document.querySelector('.sidebar-health .status-dot.good')), 'ready shell');
}

export const frontendConsistencyScenarios = [
  scenario('desktop.settings.group-navigation', ['frontend-consistency'], 'Settings groups are keyboard accessible, preserve unsaved input and do not write on navigation', async () => {
    for (const language of ['zh', 'en']) {
      const t = (zh, en) => language === 'zh' ? zh : en;
      await fresh(language); await click(t('设置', 'Settings'));
      const tabs = all('.settings-tabs [role="tab"]');
      assert(tabs.length === 3, 'Settings must expose three named areas rather than one long form');
      assert(all('[data-settings-panel]').length === 1, 'Several Settings areas are visible at once');
      await click(t('服务与接入', 'Service & access'));
      const limit = document.querySelector('input[aria-label="' + t('并发任务上限', 'Concurrent task limit') + '"]');
      await until(() => limit?.value === '10', 'current task limit');
      input(limit, '17'); await pause(50);
      await click(t('记录与诊断', 'Records & diagnostics'));
      assert(all('[data-settings-panel]').length === 1 && all('input[aria-label="' + t('记录容量上限（GB）', 'Record storage limit (GB)') + '"]').length === 1, 'Record settings are not in their named area');
      await click(t('服务与接入', 'Service & access'));
      assert(limit.value === '17', 'Switching Settings areas discarded an unsaved edit');
      const active = all('.settings-tabs [role="tab"][aria-selected="true"]')[0];
      active.focus(); active.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
      await until(() => tabs[0].getAttribute('aria-selected') === 'true', 'Home selects first Settings area');
      assert(document.activeElement === tabs[0] && all('[data-settings-panel]').length === 1, 'Keyboard selection did not move focus with the panel');
      tabs[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
      await until(() => tabs[2].getAttribute('aria-selected') === 'true', 'End selects last Settings area');
      assert(document.activeElement === tabs[2], 'Keyboard focus did not follow the last Settings area');
      assert(!control().commands.some(({ command }) => /worker_settings_set|gateway_listener_apply|set_diagnostic_level|observation_capacity_set|update_(download|install)|cli_entry_(install|remove)/.test(command)), 'Browsing Settings submitted a configuration change');
    }
  }),
  scenario('desktop.agents.capability-actions', ['frontend-consistency'], 'Agent capability actions stay together and open drafts without changing configuration', async () => {
    await fresh(); await click('Agent');
    await until(() => all('[data-agent-facet="model"]').length === 1, 'selected Agent');
    for (const facet of ['model', 'collaboration', 'management_skill']) {
      const action = all('[data-agent-facet="' + facet + '"]')[0];
      const group = action?.closest('.detail-section').querySelector('.detail-section-head');
      assert(group, `The ${facet} action is missing from its capability heading`);
      assert(action.getAttribute('role') !== 'switch', 'A confirmation-based action is presented as an immediate toggle');
      const buttons = [...group.querySelectorAll('button')];
      for (let i = 1; i < buttons.length; i++) {
        const gap = buttons[i].getBoundingClientRect().left - buttons[i - 1].getBoundingClientRect().right;
        assert(gap >= 0 && gap <= 13, `Related ${facet} actions are scattered across the heading (${gap}px)`);
      }
    }
    const edit = all('[data-agent-facet="model"]')[0]; edit.click();
    await until(() => all('[role="dialog"]').length === 1, 'Agent model draft');
    await click('取消');
    await until(() => all('[role="dialog"]').length === 0, 'cancelled Agent draft');
    assert(!control().commands.some(({ command }) => /preview_agent_settings|apply_agent|check_agent_live/.test(command)), 'Opening and cancelling an Agent draft changed or tested the connection');
  }),
];
