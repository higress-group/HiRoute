import { scenario } from './scenario-selection.mjs';
import { seedInteractionAudit, openInteractionAudit, loseInteractionAuthority } from './interaction-audit-fixtures.mjs';

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
  scenario('desktop.interactions.route-save-isolation', ['interaction-consistency'], 'A pending route save locks draft replacement but retains read-only performance navigation', async () => {
    const c=window.productShell;
    await seedInteractionAudit(c,{state:'routing',language:'en'});
    c.desktop.catalog.plans[0].desired.strategy.classifier={kind:'decision_service',service:structuredClone(c.handlers.decision_services_v3().services[0].service)};
    await openInteractionAudit('routing','en');
    all('.quality-view-tabs button').find(n=>n.textContent==='Model performance').click();
    await until(()=>all('.quality-footer-links button').length,'Division entry before save');
    all('.quality-view-tabs button')[0].click();
    const field=()=>all('.plan-identity-fields input')[0];
    await until(()=>field(),'Configuration fields');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field(),'Route saving first');
    field().dispatchEvent(new Event('input',{bubbles:true}));await pause();
    let release,submitted;
    c.handlers.preview_plan_editor=async({input})=>{
      submitted=input;await new Promise(resolve=>{release=resolve;});
      c.desktop.catalog.drafts.push({draft_id:input.draft_id,plan_id:input.plan_id,base_head_revision:input.expected_head_revision,revision:1,editor:structuredClone(input.editor)});
      return {state:'succeeded',operation:{operation_id:'operation/isolated-save',state:'succeeded',sequence:1,cancellable:false,safe_error_code:null}};
    };
    all('.plan-editor .editor-header button').find(n=>n.textContent==='Save draft').click();await until(()=>release,'Pending save');
    all('.quality-view-tabs button').find(n=>n.textContent==='Model performance').click();
    await until(()=>all('.quality-performance-panel').length,'Read-only performance during save');
    assert(!all('.quality-footer-links button').some(n=>!n.matches(':disabled')),'Pending save allows division replacement');
    const replacement=await (await import('../../../src/ui/discard-guard.ts')).requestEditorReplacement('routing');
    assert(!replacement&&!all('dialog[open]').length,'Pending save allowed editor replacement through a navigation guard');
    assert(!all('.new-plan-workspace').length,'Pending save switched to another editor');
    release();await until(()=>all('.plan-editor .editor-header button').some(n=>!n.disabled),'Save completion');
    all('.quality-view-tabs button')[0].click();
    await until(()=>field(),'Saved configuration fields');
    assert(field().value==='Route saving first'&&submitted.editor.display_name===field().value,'Save readback changed the owned draft');
    assert(writes().length===1,'Viewing performance submitted another mutation');
  }),
  scenario('desktop.interactions.guard-owner-unmount', ['interaction-consistency'], 'Removing a dialog owner releases only its pending confirmation so a new editor can still leave', async () => {
    for(const readonly of [false,true]) {
      await fresh('routing-new');
      const c=window.productShell;
      const name=all('.plan-identity-fields input')[0];
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(name,'Unmounted route draft');
      name.dispatchEvent(new Event('input',{bubbles:true}));await pause();
      const helper=await import('../../../src/ui/discard-guard.ts');
      const readonlyDecision=readonly?helper.confirmDiscard('zh'):null;
      if(!readonly) all('.routing-page .toolbar-actions button').find(n=>n.textContent==='取消').click();
      await until(()=>all('dialog[open]').length,'First owner confirmation');
      c.remount();
      await until(()=>!all('dialog[open]').length,'Removed owner dialog');
      if(readonlyDecision) assert(await readonlyDecision===false,'Unmount accepted the old discard');
      await openInteractionAudit('routing-new');
      const next=all('.plan-identity-fields input')[0];
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(next,'New owner draft');
      next.dispatchEvent(new Event('input',{bubbles:true}));await pause();
      all('.routing-page .toolbar-actions button').find(n=>n.textContent==='取消').click();
      await until(()=>all('dialog[open]').length,'Previous owner left the shared guard pending');
      all('dialog[open] button').find(n=>n.textContent==='继续编辑').click();await until(()=>!all('dialog[open]').length,'New owner keep');
      assert(next.value==='New owner draft'&&writes().length===0,'Previous owner changed or saved the new draft');
      all('.routing-page .toolbar-actions button').find(n=>n.textContent==='取消').click();await until(()=>all('dialog[open]').length,'New owner discard');
      all('dialog[open] button').find(n=>n.textContent==='不保存').click();await until(()=>!all('dialog[open]').length,'Discard');
    }
  }),
  scenario('desktop.interactions.update-download-cancel', ['interaction-consistency'], 'A pending macOS download permits cancellation while duplicate download and cancel requests are blocked', async () => {
    for (const phase of ['downloading','verifying']) {
      await fresh('settings-general');
      const c=window.productShell;
      const view={current_version:'0.5.0',phase:'available',available:{version:'0.5.1',notes:{zh:'更新说明',en:'Release notes'},size:1000},downloaded_bytes:0,active_calls:0,active_tasks:0,error:null,can_install:true,package_ready:false,recovery:null};
      let releaseDownload,releaseCancel,downloads=0,cancels=0,downloadPending=false;
      c.handlers.update_status=()=>structuredClone(view);
      c.handlers.update_download=async()=>{downloads++;downloadPending=true;view.phase=phase;await new Promise(resolve=>{releaseDownload=resolve;});downloadPending=false;return structuredClone(view);};
      c.handlers.update_cancel=async()=>{cancels++;assert(downloadPending,'Cancellation waited for the download command to finish');await new Promise(resolve=>{releaseCancel=resolve;});view.phase='available';return structuredClone(view);};
      const button=copy=>all('.settings-group button').find(n=>n.textContent===copy);
      await until(()=>button('下载更新')&&!button('下载更新').disabled,'Available update');
      button('下载更新').click();button('下载更新')?.click();
      await until(()=>button('取消升级'),'Polled download phase');
      const cancel=button('取消升级');
      assert(downloadPending&&!cancel.disabled,'A pending download disabled its cancel action');
      cancel.click();cancel.click();await until(()=>cancels===1,'Cancel request');
      assert(downloads===1&&cancel.disabled,'Download or cancellation was duplicated');
      releaseCancel();await until(()=>!button('取消升级'),'Cancelled download status');
      assert(button('下载更新').disabled,'Another download started before the original command returned');
      releaseDownload();await until(()=>!button('下载更新').disabled,'Download and cancel flags cleared');
      assert(cancels===1&&downloads===1,'Cancellation replayed a mutation');
    }
  }),
  scenario('desktop.interactions.route-dirty-cancel', ['interaction-consistency'], 'New route Cancel preserves edited input, supports guarded draft save, and keeps read-only exits non-mutating', async () => {
    for(const language of ['zh','en']) {
      const t=(zh,en)=>language==='zh'?zh:en;
      await fresh('routing-new',language);
      const c=window.productShell,field=()=>all('.plan-identity-fields input')[0];
      const cancel=()=>all('.routing-page .toolbar-actions button').find(n=>n.textContent===t('取消','Cancel'));
      const dialog=()=>all('dialog[open]')[0];
      const choose=copy=>[...dialog().querySelectorAll('button')].find(n=>n.textContent===copy).click();
      const edit=(node,value)=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(node,value);node.dispatchEvent(new Event('input',{bubbles:true}));};
      edit(field(),'Guarded route');await pause();cancel().click();
      await until(()=>dialog(),'Dirty Cancel did not ask before discarding');
      choose(t('继续编辑','Keep editing'));await until(()=>!dialog(),'Keep editing');
      assert(field().value==='Guarded route'&&writes().length===0,'Keep editing lost draft input or saved it');
      cancel().click();await until(()=>dialog(),'Discard choice');choose(t('不保存','Discard'));
      await until(()=>!all('.new-plan-workspace').length,'Discard did not close');
      assert(writes().length===0,'Discard submitted a route mutation');
      await openInteractionAudit('routing-new',language);edit(field(),'Saved on leave');
      edit(all('.plan-identity-fields input')[1],'Review draft');await pause();
      let releaseSave;
      c.handlers.preview_plan_editor=async({input})=>{
        assert(input.action==='save_draft'&&input.editor.display_name==='Saved on leave','Leave changed the submitted intent');
        await new Promise(resolve=>{releaseSave=resolve;});
        c.desktop.catalog.drafts.push({draft_id:input.draft_id,revision:1,editor:structuredClone(input.editor)});
        return {state:'succeeded',operation:{operation_id:'operation/guarded-leave',state:'succeeded',sequence:1,cancellable:false,safe_error_code:null}};
      };
      cancel().click();await until(()=>dialog(),'Save leave choice');choose(t('保存草稿并离开','Save draft and leave'));
      await until(()=>releaseSave,'Draft submission');
      assert([...dialog().querySelectorAll('.modal-foot button')].every(n=>n.disabled),'Pending draft save permits duplicate leave actions');
      releaseSave();await until(()=>!dialog()&&!all('.new-plan-workspace').length,'Confirmed draft did not close');
      assert(c.desktop.catalog.drafts.some(d=>d.editor.display_name==='Saved on leave'),'Leaving lost the saved draft');
      await openInteractionAudit('routing-new',language);edit(field(),'Read-only draft');await pause();
      const before=writes().length;cancel().click();await until(()=>dialog(),'Confirmation before authority loss');
      const staleSave=[...dialog().querySelectorAll('button')].find(n=>n.textContent===t('保存草稿并离开','Save draft and leave'));
      await loseInteractionAuthority(c);
      await until(()=>field().matches(':disabled')&&staleSave.disabled,'Open confirmation did not follow authority loss');
      staleSave.click();await pause();assert(writes().length===before,'Open confirmation used stale write authority');
      choose(t('继续编辑','Keep editing'));await until(()=>!dialog(),'Keep after authority loss');
      cancel().click();await until(()=>dialog(),'Read-only Cancel guard');
      assert(![...dialog().querySelectorAll('button')].some(n=>n.textContent===t('保存草稿并离开','Save draft and leave')),'Read-only exit offers a write');
      choose(t('继续编辑','Keep editing'));await until(()=>!dialog(),'Read-only keep');assert(field().value==='Read-only draft','Read-only keep lost input');
      cancel().click();await until(()=>dialog(),'Read-only discard');choose(t('放弃修改','Discard changes'));
      await until(()=>!all('.new-plan-workspace').length,'Read-only discard did not close');
      assert(writes().length===before,'Read-only cancellation submitted a mutation');
    }
  }),
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
