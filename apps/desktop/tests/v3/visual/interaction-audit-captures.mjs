import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

export const interactionAuditStates = ['home', 'home-empty', 'models', 'models-add', 'models-directory', 'models-credentials', 'models-credentials-discard', 'models-remove', 'models-network',
  'decision', 'decision-editor', 'decision-extension-editor', 'decision-readonly', 'decision-extension-readonly', 'decision-remove',
  'routing', 'routing-new', 'routing-quality', 'routing-mapping', 'routing-worker', 'routing-readonly', 'agent', 'agent-editor', 'agent-token', 'tasks', 'task-cancel',
  'sessions', 'sessions-facts', 'sessions-cleanup', 'sessions-quality', 'settings-general', 'settings-runtime', 'settings-records', 'settings-error', 'console-login', 'console-expired',
  'trial-setup', 'trial-footer', 'trial-readonly', 'model-defaults', 'home-suggestions', 'windows-settings', 'windows-restart', 'windows-update-busy'];

export async function interactionAuditCaptures(client, baseUrl, output, { baseline = false } = {}) {
  const results = [];
  const languages = process.env.HIROUTE_AUDIT_QUICK === '1' ? ['zh'] : ['zh', 'en'];
  const themes = process.env.HIROUTE_AUDIT_QUICK === '1' ? ['light'] : ['light', 'dark'];
  const viewports = process.env.HIROUTE_AUDIT_QUICK === '1' ? [{ width: 1200, height: 813 }] : [{ width: 1200, height: 813 }, { width: 755, height: 600 }];
  const states = process.env.HIROUTE_AUDIT_STATES?.split(',') ?? interactionAuditStates;
  // Only this dedicated, synthetic browser context receives deterministic ids/date.
  // No product source, credential or native session is modified by capture setup.
  const deterministic = await client.send('Page.addScriptToEvaluateOnNewDocument', { source: `{
    const RealDate=Date; window.Date=class extends RealDate {constructor(...args){super(...(args.length?args:['2026-11-01T01:00:00+08:00']));}};
    let sequence=0; Object.defineProperty(crypto,'getRandomValues',{value:array=>{sequence++;for(let i=0;i<array.length;i++)array[i]=(sequence*17+i*13)%256;return array;}});
  }` });
  try {
  for (const language of languages) for (const theme of themes) for (const viewport of viewports) for (const state of states) {
    const id = `interaction-${state}-${language}-${theme}-${viewport.width}`, screenshot = `${id}.png`;
    try {
      const consoleEntry = state.startsWith('console-');
      const windows = state.startsWith('windows-');
      const trial = state.startsWith('trial-'), isolated = trial || ['model-defaults','home-suggestions'].includes(state);
      const url = new URL(consoleEntry ? `console-entry.html?case=${state.slice(8)}&language=${language}&theme=${theme}`
        : windows ? `console-entry.html?case=ready&host=shell-console&update=${state==='windows-update-busy'?'waiting':'ready'}&hold-cancel=1&language=${language}&theme=${theme}`
        : isolated ? `new-model-trial.html?native=1&view=${trial?'quickstart':state==='model-defaults'?'models':'home'}&lang=${language}&theme=${theme}` : 'product-shell.html', baseUrl);
      url.searchParams.set('audit', crypto.randomUUID());
      await navigate(client, url.href, viewport);
      await waitFor(client, `location.href===${JSON.stringify(url.href)} && document.readyState==='complete'`);
      let selector;
      if (consoleEntry) {
        await waitFor(client, 'Boolean(document.querySelector(".console-entry input"))'); selector = '.console-entry-card';
      } else if (windows) {
        await waitFor(client, 'Boolean(document.querySelector(".sidebar-health .good"))');
        await evaluate(client, `[...document.querySelectorAll('.sidebar button')].find(n=>n.textContent.trim()===${JSON.stringify(language==='zh'?'设置':'Settings')}).click()`);
        await waitFor(client, 'Boolean(document.querySelector(".settings-wrap")) && window.consoleComponent.hostRequests.some(r=>r.command==="login_start_get")');
        if (state === 'windows-restart') {
          await evaluate(client, `[...document.querySelectorAll('button')].find(n=>n.textContent===${JSON.stringify(language==='zh'?'重启服务…':'Restart service…')}).click()`);
          await waitFor(client, `Boolean(document.querySelector('[role="dialog"]') || document.body.textContent.includes(${JSON.stringify(language==='zh'?'确认重启后台服务？':'Restart the background service?')}))`);
        }
        if (state === 'windows-update-busy') {
          const copy=JSON.stringify(language==='zh'?'取消升级':'Cancel update');
          await waitFor(client, `[...document.querySelectorAll('button')].some(n=>n.textContent===${copy})`);
          await evaluate(client, `(()=>{const b=[...document.querySelectorAll('button')].find(n=>n.textContent===${copy});b.scrollIntoView({block:'center'});b.click();})()`);
          await waitFor(client, 'window.consoleComponent.pendingHostCancels()===1');
        }
        selector = '.settings-wrap';
      } else if (isolated) {
        await waitFor(client, `Boolean(document.querySelector(${JSON.stringify(trial?'.new-models':state==='model-defaults'?'.models-feature':'.trust-attention')}))`);
        if (trial) {
          await waitFor(client, `[...document.querySelectorAll('button')].some(n=>n.textContent.trim()===${JSON.stringify(language==='zh'?'接入并试用':'Connect and try')}&&!n.disabled)`);
          await evaluate(client, `[...document.querySelectorAll('button')].find(n=>n.textContent.trim()===${JSON.stringify(language==='zh'?'接入并试用':'Connect and try')}).click()`);
          await waitFor(client, 'Boolean(document.querySelector(".trial-setup"))');
          if (state === 'trial-readonly') { await evaluate(client, 'window.setTrialMutable(false)'); await waitFor(client, 'Boolean(document.querySelector(".trial-setup fieldset:disabled,.trial-setup:disabled"))'); }
          if (state !== 'trial-setup') await evaluate(client, 'document.querySelector(".trial-footer").scrollIntoView({block:"end"})');
          selector = '.trial-page';
        } else selector = state==='model-defaults'?'.models-feature':'.trust-attention';
      } else {
        await waitFor(client, 'Boolean(window.productShell)');
        await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m=>m.seedInteractionAudit(window.productShell,${JSON.stringify({ language, theme, state })}))`);
        await waitFor(client, `Boolean(document.querySelector('.sidebar-health .good') && document.querySelector('.hr-ui')?.lang === ${JSON.stringify(language === 'zh' ? 'zh-CN' : 'en')})`);
        selector = await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m=>m.openInteractionAudit(${JSON.stringify(state)},${JSON.stringify(language)}))`);
      }
      await evaluate(client, `document.fonts.ready.then(()=>Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>undefined))))`);
      const geometry = await evaluate(client, `(() => {
        const root = document.querySelector(${JSON.stringify(selector)});
        if (!root?.getClientRects().length) throw Error('Missing or hidden audit surface ' + ${JSON.stringify(selector)});
        const box = node => {const b=node.getBoundingClientRect();return {left:b.left,right:b.right,top:b.top,bottom:b.bottom,width:b.width,height:b.height};};
        const visible = node => Boolean(node.getClientRects().length);
        return { root:box(root), pageOverflow:document.documentElement.scrollWidth>innerWidth+1,
          overflow:root.scrollWidth>root.clientWidth+1,
          tabs:[...root.querySelectorAll('[role=tab]')].filter(visible).map(n=>({copy:n.textContent,selected:n.getAttribute('aria-selected'),tabIndex:n.tabIndex})),
          actions:[...root.querySelectorAll('.modal-foot button,.decision-form-actions button,.toolbar-actions button,.agent-token-form button')].filter(visible).map(n=>({copy:n.textContent,disabled:n.matches(':disabled'),...box(n)})) };
      })()`);
      const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      await writeFile(path.join(output, screenshot), Buffer.from(shot.data, 'base64'));
      if (!baseline && (geometry.pageOverflow || geometry.overflow)) throw Error('Horizontal overflow: ' + JSON.stringify(geometry));
      if (state === 'trial-readonly') {
        const exit = await evaluate(client, `(() => {const buttons=[...document.querySelectorAll('.trial-footer button')];return {cancel:buttons.find(b=>/^(取消|Cancel)$/.test(b.textContent.trim()))?.matches(':disabled'),continue:buttons.find(b=>/^(检查改动|Review changes)$/.test(b.textContent.trim()))?.matches(':disabled')};})()`);
        geometry.exit = exit;
        if (!baseline && (exit.cancel !== false || exit.continue !== true)) throw Error('Read-only trial exit: '+JSON.stringify(exit));
      }
      results.push({ id, screen: state, state: 'green', baseline, language, theme, viewport, screenshot, geometry });
    } catch (error) {
      const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      await writeFile(path.join(output, screenshot), Buffer.from(shot.data, 'base64'));
      const context = await evaluate(client, '({url:location.href,copy:document.body.innerText.slice(-1200)})').catch(()=>null);
      results.push({ id, screen: state, state: 'red', baseline, language, theme, viewport, screenshot, error: String(error), context });
    }
    process.stdout.write(`${results.at(-1).state}: ${id}${results.at(-1).error ? ' · '+results.at(-1).error : ''}\n`);
  }
  } finally { await client.send('Page.removeScriptToEvaluateOnNewDocument', {identifier:deterministic.identifier}); }
  return results;
}
