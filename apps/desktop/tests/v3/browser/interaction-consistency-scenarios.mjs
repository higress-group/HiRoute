import { scenario } from './scenario-selection.mjs';
import { seedInteractionAudit, openInteractionAudit } from './interaction-audit-fixtures.mjs';

const visible = node => Boolean(node?.getClientRects().length);
const all = selector => [...document.querySelectorAll(selector)].filter(visible);
const assert = (value, message) => { if (!value) throw Error(message); };
const pause = () => new Promise(resolve => setTimeout(resolve, 25));
async function until(read, label) { for (let i=0;i<300;i++) { if(read()) return; await pause(); } throw Error(label); }
async function fresh(state, language = 'zh') {
  await seedInteractionAudit(window.productShell, {state, language});
  await until(() => all('.sidebar-health .good').length, 'Ready shell');
  return openInteractionAudit(state, language);
}
const writes = () => window.productShell.commands.filter(({command}) => /save_decision|test_decision|cli_entry_(install|remove)|apply_compute|preview_plan_editor/.test(command));

export const interactionConsistencyScenarios = [
  scenario('desktop.interactions.tab-focus-return', ['interaction-consistency'], 'Activating a model tab keeps keyboard focus on the newly visible selected tab', async () => {
    for (const language of ['zh','en']) {
      await fresh('decision',language);
      const label=language==='zh'?'通用模型':'General models';
      const target=all('.model-category-tabs button').find(n=>n.textContent===label);
      target.focus();target.click();
      await until(()=>all('.models-feature').length && !all('.decision-models-page').length,'Activated model view');
      const selected=all('.model-category-tabs button').find(n=>n.textContent===label);
      await pause();
      assert(document.activeElement===selected,'Activating a tab lost focus when its former page became hidden');
      assert(writes().length===0,'Activating a model view submitted a mutation');
    }
  }),
  scenario('desktop.interactions.credential-cancel', ['interaction-consistency'], 'Credential Cancel uses the same dirty-draft guard as closing the window and keeps persisted keys unchanged', async () => {
    await fresh('models-credentials-discard');
    const guard=all('dialog[open]')[0];
    assert(guard,'Credential Cancel silently discarded an edited draft');
    const keep=[...guard.querySelectorAll('button')].find(n=>n.textContent==='继续编辑');
    keep.click();await until(()=>!all('dialog[open]').length,'Keep editing did not dismiss guard');
    assert(all('[role="dialog"]').length===1 && !all('.oc-key-row').length,'Keep editing lost the credential draft');
    const cancel=all('[role="dialog"] .modal-foot button').find(n=>n.textContent==='取消');
    cancel.click();await until(()=>all('dialog[open]').length,'Repeated Cancel did not guard draft');
    [...all('dialog[open]')[0].querySelectorAll('button')].find(n=>n.textContent==='放弃修改').click();
    await until(()=>!all('[role="dialog"],dialog[open]').length,'Discard did not close credential editor');
    assert(writes().length===0 && window.productShell.management.sources.find(n=>n.source_id==='source/bailian/coding').keys.length===1,'Discard changed persisted credentials');
  }),
  scenario('desktop.interactions.read-only-exit', ['interaction-consistency'], 'Decision model and extension drafts can be cancelled after authority is lost without saving or testing', async () => {
    for (const state of ['decision-readonly', 'decision-extension-readonly']) {
      await fresh(state);
      const form = all('.decision-connection-form')[0], buttons = [...form.querySelectorAll('.decision-form-actions button')];
      assert(buttons.length===3 && !buttons[0].matches(':disabled'), 'Read-only mode traps the user by disabling Cancel');
      assert(buttons.slice(1).every(node=>node.matches(':disabled')), 'Read-only mode exposes a save action');
      buttons[0].click(); await until(()=>!all('.decision-connection-form').length,'Cancel did not leave read-only draft');
      assert(writes().length===0,'Cancelling a read-only draft wrote or tested configuration');
    }
  }),
  scenario('desktop.interactions.tab-navigation', ['interaction-consistency'], 'View tabs have one keyboard stop; arrow navigation preserves dirty models until explicit activation and discard', async () => {
    await fresh('decision-editor');
    const field=all('[data-native-field="name"]')[0];
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field,'Unsaved decision');
    field.dispatchEvent(new Event('input',{bubbles:true})); await pause();
    const tabs=all('.model-category-tabs [role="tab"]');
    assert(tabs.length===2 && tabs.filter(node=>node.tabIndex===0).length===1,'Model categories lack consistent keyboard tabs');
    const active=tabs.find(node=>node.getAttribute('aria-selected')==='true'); active.focus();
    active.dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true}));
    await pause();
    assert(document.activeElement===tabs[0] && all('[data-native-field="name"]')[0]?.value==='Unsaved decision','Focusing another tab changed or lost the draft');
    tabs[0].click();
    await until(()=>all('dialog[open],.route-leave-dialog').length,'Explicit navigation did not guard dirty model');
    const keep=all('button').find(node=>/继续编辑|Keep editing/.test(node.textContent));
    assert(keep,'Discard guard lacks Keep editing'); keep.click();
    await until(()=>all('[data-native-field="name"]')[0]?.value==='Unsaved decision','Keep editing lost the draft');
    assert(writes().length===0,'Keyboard navigation wrote or tested configuration');
    for (const state of ['routing','agent']) {
      await fresh(state);
      const tabs=all(state==='routing'?'.quality-view-tabs [role="tab"]':'.agent-tabs [role="tab"]');
      assert(tabs.length===2,'Equivalent view tabs lack the shared navigation pattern');
      tabs[0].focus();tabs[0].dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));await pause();
      assert(document.activeElement===tabs[1] && tabs[0].getAttribute('aria-selected')==='true','Arrow navigation activated a guarded view or lost focus');
      tabs[1].click(); await until(()=>tabs[1].getAttribute('aria-selected')==='true','Explicit activation failed');
    }
  }),
  scenario('desktop.interactions.unknown-cli', ['interaction-consistency'], 'An unavailable CLI status offers a read retry and never installs an unknown entry', async () => {
    await fresh('settings-error');
    const section=all('.settings-group').find(node=>node.querySelector('h2')?.textContent==='CLI');
    const buttons=[...section.querySelectorAll('button')];
    assert(buttons.length===1 && buttons[0].textContent==='重试' && !buttons[0].disabled,'Unknown CLI status offers a mutation instead of a read retry');
    let reads=0;window.productShell.handlers.cli_entry_status=()=>{reads++;return{state:'missing',entry_path:'/opt/hiroute/bin/hiroute',path_contains_entry_directory:false,command_resolves_to_app_cli:false,daemon_available:true};};
    buttons[0].click(); await until(()=>section.textContent.includes('尚未安装'),'Retry did not read back entry state');
    assert(reads===1 && writes().length===0,'CLI read retry submitted a write or duplicate request');
  }),
  scenario('desktop.interactions.route-actions', ['interaction-consistency'], 'Routing actions are in the page toolbar and create actions follow Cancel, draft, Enable order', async () => {
    await fresh('routing');
    const toolbar=all('.routing-page .toolbar-actions')[0];
    assert(toolbar.textContent.includes('映射 Agent'),'Mapping action breaks the page frame above the route list');
    await openInteractionAudit('routing-new');
    const buttons=all('.routing-page .toolbar-actions button');
    assert(buttons[0].textContent==='取消' && buttons.at(-1).textContent==='启用','Create route actions differ from other editors');
    buttons[0].click(); await until(()=>!all('.new-plan-workspace').length,'Cancel did not close a clean route draft');
    assert(writes().length===0,'Opening and cancelling a route persisted configuration');
  }),
  scenario('desktop.interactions.read-only-routing', ['interaction-consistency'], 'Losing write authority disables route edits and publication while retaining view navigation', async () => {
    await fresh('routing-readonly');
    const fields=all('.plan-identity-fields input');
    assert(fields.length===2 && fields.every(n=>n.matches(':disabled')),'Read-only routing leaves editable fields');
    assert(all('.plan-editor .editor-header button').every(n=>n.matches(':disabled')),'Read-only routing exposes publication');
    const performance=all('.quality-view-tabs button').find(n=>n.textContent==='模型表现');
    assert(performance && !performance.matches(':disabled'),'Read-only routing disables view navigation');
    performance.click(); await until(()=>all('.quality-performance-panel').length,'Read-only performance view did not open');
    assert(writes().length===0,'Read-only view navigation submitted a mutation');
  }),
  scenario('desktop.interactions.diagnostics-save', ['interaction-consistency'], 'One log-level draft saves explicitly with its accepted revision and prevents duplicate submission', async () => {
    await fresh('settings-records');
    const section=all('.settings-group').find(n=>n.querySelector('#diagnostics-heading'));
    assert(section && !section.querySelector('input[type=checkbox]'),'Two controls compete for the same log-level draft');
    const select=section.querySelector('select');
    await until(()=>window.productShell.commands.some(c=>c.command==='diagnostic_status'),'Diagnostic status');
    select.value='debug';select.dispatchEvent(new Event('change',{bubbles:true}));await pause();
    let count=0,release;
    window.productShell.handlers.set_diagnostic_level=async({input})=>{
      count++;assert(input.expected_revision===1 && input.level==='debug','Log save lost its accepted revision or draft');
      await new Promise(resolve=>{release=resolve;});
      return {schema:'hiroute.diagnostics-status/v1',settings:{revision:2,level:'debug'},logs_directory_available:true};
    };
    const save=[...section.querySelectorAll('button')].find(n=>n.textContent==='保存等级');
    await until(()=>!save.disabled,'Changed log level can be saved');
    assert(count===0,'Choosing a log level saved without the explicit action');
    save.click();await until(()=>count===1,'Log save');
    assert(select.disabled && save.disabled,'Pending log save still accepts changes or duplicate submission');
    save.click();assert(count===1,'Log save was duplicated');
    release();await until(()=>!select.disabled && select.value==='debug' && save.disabled,'Saved log level was not retained');
  }),
];
