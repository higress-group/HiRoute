import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { evaluate,navigate,waitFor } from './cdp-client.mjs';
export async function runShellHostScenarios(client,baseUrl,output) {
  const results=[];
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const screenshot=async(name)=>{const shot=await client.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(path.join(output,name),Buffer.from(shot.data,'base64'));return name;};
  const open=async(language,theme,width,height,update='ready',state='ready',extra={})=>{
    const url=new URL('console-entry.html',baseUrl);
    for(const [key,value]of Object.entries({case:state,host:'shell-console',language,theme,update,...extra}))url.searchParams.set(key,value);
    await navigate(client,url.href,{width,height});
    try { await waitFor(client,'Boolean(window.consoleComponent)'); }
    catch(error) {
      await writeFile(path.join(output,'shell-entry-failure.json'),JSON.stringify(await evaluate(client,'({url:location.href,ready:document.readyState,text:document.body.innerText,resources:performance.getEntriesByType("resource").map(r=>({name:r.name,status:r.responseStatus,duration:r.duration}))})'),null,2));
      await screenshot('shell-entry-failure.png');throw error;
    }
    if(state==='expired')return;
    await waitFor(client,'Boolean(document.querySelector(".sidebar"))');
    await evaluate(client,"[...document.querySelectorAll('.sidebar-foot .nav-item')].find(button=>button.getClientRects().length).click()");
    try { await waitFor(client,'document.querySelector(".settings-group")?.textContent.includes("Windows") && Boolean(document.querySelector(".settings-group button[aria-pressed]"))'); }
    catch(error) { await writeFile(path.join(output,'shell-ui-failure.json'),JSON.stringify(await evaluate(client,'({text:document.body.innerText,capabilities:window.consoleComponent.capabilities(),requests:window.consoleComponent.hostRequests,errors:window.consoleComponent.errors})'),null,2));await screenshot('shell-ui-failure.png');throw error; }
    await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="update_status")');
    const page=await evaluate(client,'(()=>{const b=document.querySelector(".main .page").getBoundingClientRect();return{banner:!!document.querySelector(".console-transport-bar"),top:b.top,bottom:b.bottom};})()');
    assert(!page.banner&&page.top<90&&page.bottom<=height+1,'Windows host notice displaced the product page: '+JSON.stringify(page));
  };

  const updateUrl = new URL('console-entry.html?case=ready&host=shell-console&native-shell=1&update=discover&language=en&theme=dark', baseUrl);
  await navigate(client, updateUrl.href, { width: 755, height: 600 });
  await waitFor(client, "Boolean(document.querySelector('.sidebar-settings')?.getClientRects().length)");
  await evaluate(client, `(() => {
    window.settingsGearBeforeDiscovery = document.querySelector('.sidebar-settings');
    document.querySelector('.sidebar .nav button:last-child').focus();
  })()`);
  await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
  await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
  assert(await evaluate(client, `document.activeElement === window.settingsGearBeforeDiscovery
    && document.activeElement.getAttribute('aria-label') === 'Settings'
    && document.activeElement.matches(':focus-visible')`),
    'The unmarked Settings gear did not take focus');
  await screenshot('shell-update-sidebar-focused-before-discovery-en-dark-755.png');
  await waitFor(client, "window.consoleComponent.hostRequests.some(({command}) => command === 'update_check')");
  await evaluate(client, 'window.consoleComponent.releaseUpdateDiscovery()');
  await waitFor(client, "Boolean(document.querySelector('.sidebar-update-dot')?.getClientRects().length)");
  assert(await evaluate(client, `(() => {
    const gear = window.settingsGearBeforeDiscovery;
    return document.activeElement === gear && gear.matches(':focus-visible') && gear.getClientRects().length
      && gear.getAttribute('aria-label') === 'Settings · New version available'
      && gear.querySelector('.sidebar-update-dot')?.getClientRects().length
      && !document.querySelector('.sidebar-update-entry').getClientRects().length
      && window.consoleComponent.hostRequests.filter(({command}) => command === 'update_check').length === 1;
  })()`), 'Update discovery replaced the compact Settings gear or lost its keyboard focus');
  await screenshot('shell-update-sidebar-en-dark-755.png');

  await client.send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 813, deviceScaleFactor: 1, mobile: false });
  await waitFor(client, `window.settingsGearBeforeDiscovery.getAttribute('aria-label') === 'Settings'
    && Boolean(document.querySelector('.sidebar-update-entry')?.getClientRects().length)`);
  assert(await evaluate(client, 'document.activeElement === window.settingsGearBeforeDiscovery'),
    'Expanding the sidebar lost focus on Settings');
  await client.send('Emulation.setDeviceMetricsOverride', { width: 755, height: 600, deviceScaleFactor: 1, mobile: false });
  await waitFor(client, "window.settingsGearBeforeDiscovery.getAttribute('aria-label') === 'Settings · New version available'");
  assert(await evaluate(client, 'document.activeElement === window.settingsGearBeforeDiscovery'),
    'Collapsing the sidebar lost focus on Settings');

  await client.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' });
  await client.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  await waitFor(client, `(() => {
    const heading = document.querySelector('.settings-updates h2');
    return document.activeElement === heading
      && heading.getBoundingClientRect().top >= heading.closest('.page-content').getBoundingClientRect().top;
  })()`);
  assert(await evaluate(client, `document.querySelector('.settings-updates').innerText.includes('Version 0.4.0 is available')
    && !window.consoleComponent.requests.some(({path}) => path.includes('update_'))
    && !window.consoleComponent.hostRequests.some(({command}) => ['update_download', 'update_install', 'update_cancel'].includes(command))`),
    'The Windows shortcut changed updater state or sent a maintenance command through console HTTP');
  const updateScreenshot = await screenshot('shell-update-destination-en-dark-755.png');
  results.push({ id: 'shell.sidebar-update-entry', state: 'green', screenshot: updateScreenshot });

  for(const language of ['zh','en'])for(const theme of ['light','dark'])for(const [width,height]of [[1200,813],[755,600]]) {
    for(const phase of ['ready','waiting','failed','catalog-unavailable']){
      await open(language,theme,width,height,phase);
      const box={panels:[]};
      for(const section of ['general','runtime','records']) {
        await evaluate(client,`document.querySelector('.settings-tabs button[aria-controls$="-panel-${section}"]').click()`);
        await waitFor(client,`!document.querySelector('[data-settings-panel="${section}"]').hidden`);
        const panel=await evaluate(client,`(()=>{const wrap=document.querySelector('.settings-wrap');const panel=wrap.querySelector('[data-settings-panel="${section}"]');const rows=[...panel.querySelectorAll('.settings-row')];return {section:${JSON.stringify(section)},overflow:wrap.scrollWidth>wrap.clientWidth+1,rows:rows.map(e=>{const b=e.getBoundingClientRect();return{width:b.width,overflow:e.scrollWidth>e.clientWidth+1};})};})()`);
        assert(!panel.overflow && panel.rows.length>0 && panel.rows.every(r=>r.width>200&&!r.overflow),'Shell settings overflow: '+JSON.stringify(panel));
        box.panels.push(panel);
      }
      await evaluate(client,'document.querySelector(\'.settings-tabs button[aria-controls$="-panel-general"]\').click()');
      await waitFor(client,'!document.querySelector(\'[data-settings-panel="general"]\').hidden');
      if(phase==='ready') {
        const hostScreenshot=await screenshot(`shell-host-${language}-${theme}-${width}x${height}.png`);
        results.push({id:`shell.host.${language}.${theme}.${width}`,state:'green',screenshot:hostScreenshot});
      }
      await evaluate(client,`[...document.querySelectorAll('.settings-group')].find(e=>e.querySelector('h2')?.textContent===${JSON.stringify(language==='zh'?'版本升级':'Updates')}).scrollIntoView({block:'start'})`);
      if(phase==='catalog-unavailable') {
        const state=await evaluate(client,`(()=>{const group=[...document.querySelectorAll('.settings-group')].find(e=>e.querySelector('h2')?.textContent===${JSON.stringify(language==='zh'?'版本升级':'Updates')});return{text:group.textContent,buttons:[...group.querySelectorAll('button')].map(b=>({text:b.textContent,disabled:b.disabled}))};})()`);
        assert(state.text.includes('UPGRADE_CATALOG_UNAVAILABLE') && state.text.includes(language==='zh'?'Windows 更新目录暂不可用':'Windows update catalog is unavailable'),'Missing catalog was not identified');
        assert(!state.text.includes(language==='zh'?'升级未完成':'upgrade did not complete') && !state.text.includes(language==='zh'?'最新稳定版本':'latest stable version'),'Failed check implied installation failure or a completed check');
        assert(state.buttons.length===1&&!state.buttons[0].disabled,'Failed check did not retain exactly one usable retry action');
      }
      const name=await screenshot(`shell-settings-${phase}-${language}-${theme}-${width}x${height}.png`);
      results.push({id:`shell.settings.${phase}.${language}.${theme}.${width}`,state:'green',screenshot:name,geometry:box});
    }
  }
  await open('en','light',755,600,'failed');
  assert(await evaluate(client,'document.body.innerText.includes("open-update-installer") && document.body.innerText.includes("Win32 32")'),'Native update failure lost its operation or Windows code');
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Discard package").click()');
  await waitFor(client,'document.body.innerText.includes("Download update") && !document.body.innerText.includes("Discard package") && !document.body.innerText.includes("Win32 32")');
  assert(await evaluate(client,'window.consoleComponent.hostRequests.filter(r=>r.command==="update_cancel").length===1'),'Discard submitted duplicate host requests');
  results.push({id:'shell.discard-failed-stage-and-retry',state:'green'});
  await open('en','light',1200,813,'waiting');
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Cancel update").click()');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="update_cancel")');
  assert(await evaluate(client,'!window.consoleComponent.requests.some(r=>r.path.includes("update_"))'),'Shell updater escaped onto console HTTP');
  await evaluate(client,'[...document.querySelectorAll(".settings-row")].find(e=>e.textContent.includes("Start at login")).querySelector("button").click()');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="login_start_set"&&r.args.enabled===false)');
  results.push({id:'shell.host-command-routing',state:'green'});
  await open('zh','light',1200,813,'ready','ready',{'native-shell':'1'});
  await evaluate(client,`document.querySelector('.settings-tabs button[aria-controls$="-panel-records"]').click()`);
  const showPath = '[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="显示目录路径"&&!b.disabled)';
  await waitFor(client,`Boolean(${showPath})`);
  await evaluate(client,`(${showPath}).click()`);
  await waitFor(client,'Boolean(document.querySelector("[role=dialog] input"))');
  assert(await evaluate(client,'document.querySelector("[role=dialog] input").value===window.consoleComponent.windowsDiagnosticDirectory'),'Windows must show the authenticated console path');
  const diagnosticScreenshot = await screenshot('shell-diagnostic-directory.png');
  results.push({id:'shell.diagnostic-directory',state:'green',screenshot:diagnosticScreenshot});
  await open('en','light',1200,813,'waiting','ready',{'hold-cancel':'1'});
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Cancel update").click()');
  await waitFor(client,'window.consoleComponent.pendingHostCancels()===1');
  assert(await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Cancel update").disabled'),'Pending update cancellation can be submitted again');
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Cancel update").click()');
  assert(await evaluate(client,'window.consoleComponent.hostRequests.filter(r=>r.command==="update_cancel").length===1'),'Update cancellation was duplicated');
  await evaluate(client,'window.consoleComponent.releaseHostCancels()');
  await waitFor(client,'!document.body.innerText.includes("Cancel update")');
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Restart service…").click()');
  await waitFor(client,'Boolean(document.querySelector("[role=dialog]"))');
  assert(await evaluate(client,'document.querySelector("[role=dialog] .modal-foot button").textContent==="Cancel"'),'Restart confirmation differs from shared action order');
  await evaluate(client,'document.querySelector("[role=dialog] .modal-foot button").click()');
  await waitFor(client,'!document.querySelector("[role=dialog]")');
  assert(await evaluate(client,'!window.consoleComponent.hostRequests.some(r=>r.command==="restart_service")'),'Cancelling the restart changed the service');
  results.push({id:'shell.pending-cancel-and-restart-dismiss',state:'green'});
  await open('en','light',1200,813,'ready','expired');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="reconnect")');
  assert(await evaluate(client,'!document.querySelector("input[name=access-token]")&&!sessionStorage.getItem("hiroute.console.session")'),'Shell expiry exposed a token login or retained its bearer');
  results.push({id:'shell.expiry-remints-through-host',state:'green'});
  const shellIndex=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../../../shell/index.html');
  for(const language of ['zh','en'])for(const theme of ['light','dark'])for(const [width,height]of [[1200,813],[755,600]])for(const phase of ['waiting','failed','listener-unavailable']){
    const code=phase==='listener-unavailable'?'CONSOLE_LISTENER_UNAVAILABLE':phase==='failed'?'SHELL_INSTALL_RECOVERY_REQUIRED':null;
    const script=await client.send('Page.addScriptToEvaluateOnNewDocument',{source:`Object.defineProperty(navigator,'language',{value:${JSON.stringify(language)}});window.shellHostRequests=[];window.__TAURI_INTERNALS__={invoke:async(command)=>{window.shellHostRequests.push(command);return command==='shell_status'?{phase:${JSON.stringify(phase==='listener-unavailable'?'failed':phase)},code:${JSON.stringify(code)},logs_path:'C:\\\\Users\\\\example\\\\AppData\\\\Local\\\\HiRoute\\\\cli\\\\diagnostics'}:null}};`});
    await client.send('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme}]});
    try{
      await navigate(client,new URL('/@fs'+shellIndex,baseUrl).href,{width,height});await waitFor(client,'Boolean(document.querySelector("#explanation")?.textContent)');
      const box=await evaluate(client,`(()=>{const e=document.querySelector('.shell-status section'),b=e.getBoundingClientRect();return{left:b.left,right:b.right,overflow:e.scrollWidth>e.clientWidth+1,theme:document.querySelector('#root').dataset.theme,button:[...document.querySelectorAll('button')].filter(b=>!b.hidden).map(b=>({width:b.getBoundingClientRect().width,height:b.getBoundingClientRect().height}))};})()`);
      assert(box.left>=0&&box.right<=width+1&&!box.overflow&&box.theme===theme&&box.button.every(b=>b.width>35&&b.height>25),'Shell startup layout failed: '+JSON.stringify(box));
      if(phase==='failed') {
        const state=await evaluate(client,`({heading:document.querySelector('#phase').textContent,text:document.querySelector('#explanation').textContent,retry:!document.querySelector('#retry').hidden,recovery:!document.querySelector('#recovery-material').hidden})`);
        assert(state.heading===(language==='zh'?'安装尚未完成':'Installation is incomplete')&&!state.retry&&!state.recovery,'Installer recovery borrowed service/storage failure actions: '+JSON.stringify(state));
        assert(state.text.includes(language==='zh'?'相同版本及构建':'same version and build')&&state.text.includes(language==='zh'?'寻求支持':'obtain support'),'Installer recovery did not identify the exact retry or missing-installer fallback');
      }
      if(phase==='listener-unavailable') {
        const state=await evaluate(client,`(()=>{const recovery=document.querySelector('#console-recovery');const main=document.querySelector('.shell-status');main.scrollTop=0;return{heading:document.querySelector('#phase').textContent,visible:recovery&&!recovery.hidden,steps:recovery?.textContent,top:document.querySelector('h1').getBoundingClientRect().top,commands:recovery?.querySelector('pre')?.textContent};})()`);
        assert(state.visible&&state.heading===(language==='zh'?'本机控制台尚未就绪':'The local console is unavailable')&&state.top>=0,'Healthy service was described as a startup failure or clipped: '+JSON.stringify(state));
        assert(state.commands==='hiroute console init --mode loopback\nhiroute service restart' && state.steps.includes(language==='zh'?'可能中断':'may interrupt'),'Missing explicit configuration/restart steps or restart warning');
        await evaluate(client,'document.querySelector("#retry").scrollIntoView({block:"end"});document.querySelector("#retry").click()');
        await waitFor(client,'window.shellHostRequests.includes("shell_retry_startup")');
        const action=await evaluate(client,'(()=>{const b=document.querySelector("#retry").getBoundingClientRect();return{top:b.top,bottom:b.bottom,commands:window.shellHostRequests.filter(c=>c!=="shell_status")};})()');
        assert(action.top>=0&&action.bottom<=height+1&&action.commands.every(c=>c==='shell_retry_startup'),'Recheck was clipped or bypassed the fixed startup path: '+JSON.stringify({language,theme,width,height,action}));
      }
      const name=await screenshot(`shell-startup-${phase}-${language}-${theme}-${width}x${height}.png`);
      results.push({id:`shell.startup.${phase}.${language}.${theme}.${width}`,state:'green',screenshot:name,geometry:box});
    }finally{await client.send('Page.removeScriptToEvaluateOnNewDocument',{identifier:script.identifier});}
  }
  await writeFile(path.join(output,'shell-host-report.json'),JSON.stringify({evidence:'Production console/settings and actual shell HTML/styles with synthetic host replies. Native origin ACL, installation and update acceptance remain separate.',results},null,2)+'\n');
  for(const result of results)process.stdout.write(`green: ${result.id}\n`);
}
