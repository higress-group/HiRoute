import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { evaluate,navigate,waitFor } from './cdp-client.mjs';
export async function runShellHostScenarios(client,baseUrl,output) {
  const results=[];
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const screenshot=async(name)=>{const shot=await client.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(path.join(output,name),Buffer.from(shot.data,'base64'));return name;};
  const open=async(language,theme,width,height,update='ready',state='ready')=>{
    const url=new URL('console-entry.html',baseUrl);
    for(const [key,value]of Object.entries({case:state,host:'shell-console',language,theme,update}))url.searchParams.set(key,value);
    await navigate(client,url.href,{width,height});await waitFor(client,'Boolean(window.consoleComponent)');
    if(state==='expired')return;
    await waitFor(client,'Boolean(document.querySelector(".sidebar"))');
    await evaluate(client,`[...document.querySelectorAll('.sidebar button')].find(b=>b.textContent.trim()===${JSON.stringify(language==='zh'?'设置':'Settings')}).click()`);
    try { await waitFor(client,'document.querySelector(".settings-group")?.textContent.includes("Windows") && Boolean(document.querySelector(".settings-group button[aria-pressed]"))'); }
    catch(error) { await writeFile(path.join(output,'shell-ui-failure.json'),JSON.stringify(await evaluate(client,'({text:document.body.innerText,capabilities:window.consoleComponent.capabilities(),requests:window.consoleComponent.hostRequests,errors:window.consoleComponent.errors})'),null,2));await screenshot('shell-ui-failure.png');throw error; }
    await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="update_status")');
  };
  for(const language of ['zh','en'])for(const theme of ['light','dark'])for(const [width,height]of [[1200,813],[755,600]]) {
    for(const phase of ['ready','waiting','failed']){
      await open(language,theme,width,height,phase);
      const box=await evaluate(client,`(()=>{const wrap=document.querySelector('.settings-wrap');const rows=[...wrap.querySelectorAll('.settings-row')];return {overflow:wrap.scrollWidth>wrap.clientWidth+1,rows:rows.map(e=>{const b=e.getBoundingClientRect();return{width:b.width,overflow:e.scrollWidth>e.clientWidth+1};})};})()`);
      assert(!box.overflow && box.rows.every(r=>r.width>200&&!r.overflow),'Shell settings overflow: '+JSON.stringify(box));
      if(phase==='ready') {
        const hostScreenshot=await screenshot(`shell-host-${language}-${theme}-${width}x${height}.png`);
        results.push({id:`shell.host.${language}.${theme}.${width}`,state:'green',screenshot:hostScreenshot});
      }
      await evaluate(client,`[...document.querySelectorAll('.settings-group')].find(e=>e.querySelector('h2')?.textContent===${JSON.stringify(language==='zh'?'版本升级':'Updates')}).scrollIntoView({block:'start'})`);
      const name=await screenshot(`shell-settings-${phase}-${language}-${theme}-${width}x${height}.png`);
      results.push({id:`shell.settings.${phase}.${language}.${theme}.${width}`,state:'green',screenshot:name,geometry:box});
    }
  }
  await open('en','light',1200,813,'waiting');
  await evaluate(client,'[...document.querySelectorAll("button")].find(b=>b.textContent==="Cancel update").click()');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="update_cancel")');
  assert(await evaluate(client,'!window.consoleComponent.requests.some(r=>r.path.includes("update_"))'),'Shell updater escaped onto console HTTP');
  await evaluate(client,'[...document.querySelectorAll(".settings-row")].find(e=>e.textContent.includes("Start at login")).querySelector("button").click()');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="login_start_set"&&r.args.enabled===false)');
  results.push({id:'shell.host-command-routing',state:'green'});
  await open('en','light',1200,813,'ready','expired');
  await waitFor(client,'window.consoleComponent.hostRequests.some(r=>r.command==="reconnect")');
  assert(await evaluate(client,'!document.querySelector("input[name=access-token]")&&!sessionStorage.getItem("hiroute.console.session")'),'Shell expiry exposed a token login or retained its bearer');
  results.push({id:'shell.expiry-remints-through-host',state:'green'});
  const shellIndex=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../../../shell/index.html');
  for(const language of ['zh','en'])for(const theme of ['light','dark'])for(const [width,height]of [[1200,813],[755,600]])for(const phase of ['waiting','failed']){
    const script=await client.send('Page.addScriptToEvaluateOnNewDocument',{source:`Object.defineProperty(navigator,'language',{value:${JSON.stringify(language)}});window.__TAURI_INTERNALS__={invoke:async(command)=>command==='shell_status'?{phase:${JSON.stringify(phase)},code:${phase==='failed'?JSON.stringify('SHELL_INSTALL_RECOVERY_REQUIRED'):'null'},logs_path:'C:\\\\Users\\\\example\\\\AppData\\\\Local\\\\HiRoute\\\\cli\\\\diagnostics'}:null};`});
    await client.send('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme}]});
    try{
      await navigate(client,new URL('/@fs'+shellIndex,baseUrl).href,{width,height});await waitFor(client,'Boolean(document.querySelector("#explanation")?.textContent)');
      const box=await evaluate(client,`(()=>{const e=document.querySelector('.shell-status section'),b=e.getBoundingClientRect();return{left:b.left,right:b.right,overflow:e.scrollWidth>e.clientWidth+1,theme:document.querySelector('#root').dataset.theme,button:[...document.querySelectorAll('button')].filter(b=>!b.hidden).map(b=>({width:b.getBoundingClientRect().width,height:b.getBoundingClientRect().height}))};})()`);
      assert(box.left>=0&&box.right<=width+1&&!box.overflow&&box.theme===theme&&box.button.every(b=>b.width>35&&b.height>25),'Shell startup layout failed: '+JSON.stringify(box));
      const name=await screenshot(`shell-startup-${phase}-${language}-${theme}-${width}x${height}.png`);
      results.push({id:`shell.startup.${phase}.${language}.${theme}.${width}`,state:'green',screenshot:name,geometry:box});
    }finally{await client.send('Page.removeScriptToEvaluateOnNewDocument',{identifier:script.identifier});}
  }
  await writeFile(path.join(output,'shell-host-report.json'),JSON.stringify({evidence:'Production console/settings and actual shell HTML/styles with synthetic host replies. Native origin ACL, installation and update acceptance remain separate.',results},null,2)+'\n');
  for(const result of results)process.stdout.write(`green: ${result.id}\n`);
}
