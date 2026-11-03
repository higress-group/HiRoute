import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Production components/styles, controlled IPC. No native save or latency claim.
export async function codexDesktopPlatformCaptures(client, baseUrl, output) {
  const results=[];
  for (const platform of ['windows','linux']) for (const [language,theme] of [['zh','dark'],['en','light']]) {
    for (const viewport of [{width:1200,height:813},{width:755,height:600}]) {
      await navigate(client,new URL(`agent-trust.html?language=${language}&theme=${theme}`,baseUrl).href,viewport);
      await waitFor(client,'Boolean(window.agentTrust && document.querySelector(".native-list .list-row"))');
      await evaluate(client,"import('/agent-trust-scenarios.mjs').then(m=>m.showCodexIndependentDesktop())");
      await evaluate(client,`window.agentTrust.setPlatform(${JSON.stringify(platform)}); window.agentTrust.refresh()`);
      await waitFor(client,`Boolean(document.querySelector('[data-codex-desktop-launch]') && !document.querySelector('[data-codex-desktop-launch]').disabled && ${platform==='linux' ? "document.querySelector('.codex-instance-command')" : "!document.querySelector('.codex-instance-command')"})`);
      if (platform==='linux') {
        await evaluate(client,"document.querySelector('.codex-instance-command summary').click()");
        await waitFor(client,"Boolean(document.querySelector('.codex-instance-command details[open]'))");
      }
      const geometry=await evaluate(client,`(() => {
        const box=document.querySelector('[data-codex-desktop-instance]'), action=box.querySelector('[data-codex-desktop-launch]'), rect=action.getBoundingClientRect();
        const pre=box.querySelector('pre');
        return {overflow:box.scrollWidth>box.clientWidth+1,bodyOverflow:document.documentElement.scrollWidth>innerWidth+1,
          actionVisible:rect.left>=0 && rect.right<=innerWidth+1 && rect.top>=0 && rect.bottom<=innerHeight,
          commandOverflow:pre ? pre.scrollWidth>pre.clientWidth+1 : false,text:box.innerText};
      })()`);
      if (geometry.overflow || geometry.bodyOverflow || geometry.commandOverflow || !geometry.actionVisible) throw Error(`Platform launch geometry: ${JSON.stringify({platform,language,viewport,geometry})}`);
      const id=`codex-desktop-${platform}-${language}-${theme}-${viewport.width}`;
      const shot=await client.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
      await writeFile(path.join(output,`${id}.png`),Buffer.from(shot.data,'base64'));
      results.push({id,state:'green',platform,language,theme,viewport,geometry,screenshot:`${id}.png`});
    }
  }
  return results;
}

export async function agentReadbackCaptures(client, baseUrl, output) {
  const results = [];
  for (const [language, theme] of [['zh', 'dark'], ['en', 'light']]) {
    for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await navigate(client, new URL(`agent-trust.html?language=${language}&theme=${theme}`, baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.agentTrust && document.querySelector(".native-list .list-row"))');
      await evaluate(client, `Array.from(document.querySelectorAll('.native-list .list-row')).find(row => row.textContent.includes('Claude')).click()`);
      await waitFor(client, 'Boolean(document.querySelector("[data-agent-id=agent_claude_default] [data-agent-facet=model]"))');
      await evaluate(client, `document.querySelector('[data-agent-facet=model]').click()`);
      await waitFor(client, 'Boolean(document.querySelector("[role=dialog] button[type=submit]"))');
      await evaluate(client, `document.querySelector('[role=dialog] button[type=submit]').click()`);
      await waitFor(client, 'Boolean(document.querySelector("[data-agent-skills-unverified]") && !document.querySelector("[data-agent-facet=model]").disabled)');
      for (const facet of ['collaboration', 'management']) {
        const selector = `[data-agent-${facet}]`;
        await evaluate(client, `document.querySelector(${JSON.stringify(selector)}).scrollIntoView({ block: 'center' })`);
        const geometry = await evaluate(client, `(() => {
          const box = document.querySelector(${JSON.stringify(selector)}), button = box.querySelector('button');
          const b = box.getBoundingClientRect(), a = button.getBoundingClientRect();
          return { overflow: box.scrollWidth > box.clientWidth + 1, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
            actionVisible: a.left >= 0 && a.right <= innerWidth + 1 && a.top >= 0 && a.bottom <= innerHeight,
            width: b.width, text: box.innerText, labelHeight: box.querySelector('h3').getBoundingClientRect().height };
        })()`);
        if (geometry.overflow || geometry.bodyOverflow || !geometry.actionVisible || geometry.labelHeight > 60) throw Error(`Agent readback geometry: ${JSON.stringify({ language, viewport, geometry })}`);
        const id = `agent-readback-${facet}-${language}-${theme}-${viewport.width}`;
        const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
        await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
        results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
      }
    }
  }
  return results;
}


export async function dshDesktopCaptures(client, baseUrl, output) {
  const results = [];
  for (const [language, theme] of [['zh', 'dark'], ['en', 'light']]) {
    for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await navigate(client, new URL(`agent-trust.html?language=${language}&theme=${theme}`, baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.agentTrust && document.querySelector(".native-list .list-row"))');
      await evaluate(client, `import('/agent-trust-scenarios.mjs').then(m => m.showDshDesktopEditor())`);
      const geometry = await evaluate(client, `(() => {
        const box = document.querySelector('[role=dialog]'), button = box.querySelector('button[type=submit]');
        const a = button.getBoundingClientRect();
        return { overflow: box.scrollWidth > box.clientWidth + 1,
          bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          actionVisible: a.left >= 0 && a.right <= innerWidth + 1 && a.top >= 0 && a.bottom <= innerHeight,
          text: box.innerText };
      })()`);
      if (geometry.overflow || geometry.bodyOverflow || !geometry.actionVisible) throw Error(`DSH Desktop geometry: ${JSON.stringify({ language, viewport, geometry })}`);
      const id = `dsh-desktop-${language}-${theme}-${viewport.width}`;
      const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
      results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
    }
  }
  return results;
}

export async function codexDesktopCaptures(client, baseUrl, output) {
  const results = [];
  for (const [language, theme] of [['zh', 'dark'], ['en', 'dark'], ['zh', 'light'], ['en', 'light']]) {
    for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await navigate(client, new URL(`agent-trust.html?language=${language}&theme=${theme}`, baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.agentTrust && document.querySelector(".native-list .list-row"))');
      for (const state of ['unconfigured', 'configured', 'launch-error', 'editor']) {
        if (state === 'unconfigured' || state === 'configured') await evaluate(client, `import('/agent-trust-scenarios.mjs').then(m => m.showCodexIndependentDesktop(${state !== 'unconfigured'}))`);
        if (state === 'launch-error') {
          await evaluate(client, `window.agentTrust.handlers.launch_codex_desktop = () => { throw { code: 'CODEX_DESKTOP_LAUNCH_FAILED' }; }; document.querySelector('[data-codex-desktop-launch]').click()`);
          await waitFor(client, 'Boolean(document.querySelector("[data-error-code=CODEX_DESKTOP_LAUNCH_FAILED]"))');
        }
        if (state === 'editor') {
          await evaluate(client, `document.querySelector('[data-agent-facet=model]').click()`);
          await waitFor(client, 'Boolean(document.querySelector("[role=dialog]"))');
        }
        const geometry = await evaluate(client, `(() => {
          const box = document.querySelector('[role=dialog]') || document.querySelector('[data-codex-desktop-instance]');
          const button = box.querySelector('button[type=submit]') || box.querySelector('[data-codex-desktop-launch], [data-codex-desktop-configure]');
          const a = button.getBoundingClientRect();
          const luminance = color => {
            const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
            const context = canvas.getContext('2d'); context.fillStyle = color; context.fillRect(0, 0, 1, 1);
            return [...context.getImageData(0, 0, 1, 1).data].slice(0, 3).map(v => v / 255)
              .map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4)
              .reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
          };
          const style = getComputedStyle(button), foreground = luminance(style.color), background = luminance(style.backgroundColor);
          const actionContrast = document.querySelector('[role=dialog]') ? null : (Math.max(foreground, background) + .05) / (Math.min(foreground, background) + .05);
          return { overflow: box.scrollWidth > box.clientWidth + 1, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
            actionVisible: a.left >= 0 && a.right <= innerWidth + 1 && a.top >= 0 && a.bottom <= innerHeight,
            actionContrast, text: box.innerText };
        })()`);
        if (geometry.overflow || geometry.bodyOverflow || !geometry.actionVisible || geometry.actionContrast !== null && geometry.actionContrast < 4.5) throw Error(`Codex Desktop geometry: ${JSON.stringify({ language, viewport, state, geometry })}`);
        const id = `codex-desktop-${state}-${language}-${theme}-${viewport.width}`;
        const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
        await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
        results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
      }
    }
  }
  return results;
}

export async function agentDetailsCaptures(client, baseUrl, output) {
  const results = [];
  const ids = ['agent_claude_default', 'agent_codex_default', 'agent_qoder_default', 'agent_hermes_default',
    'agent_pi_default', 'agent_dsh_default', 'agent_dsh_desktop', 'agent_opencode_default'];
  for (const [language, theme] of [['zh', 'dark'], ['en', 'light']]) {
    for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
      await navigate(client, new URL(`agent-trust.html?language=${language}&theme=${theme}`, baseUrl).href, viewport);
      await waitFor(client, 'Boolean(window.agentTrust && document.querySelector(".native-list .list-row"))');
      for (const agent of ids) {
        await evaluate(client, `import('/agent-trust-scenarios.mjs').then(m => m.showAgentDetail(${JSON.stringify(agent)}))`);
        const geometry = await evaluate(client, `(async () => {
          let scrollProof = null;
          if (innerWidth === 755 && document.querySelector('.agent-detail').dataset.agentId === 'agent_claude_default') {
            const pane = document.querySelector('.detail-pane');
            pane.scrollTop = pane.scrollHeight;
            const before = pane.scrollTop;
            const select = name => [...document.querySelectorAll('.native-list .list-row')]
              .find(row => row.querySelector('.row-title')?.textContent.trim() === name).click();
            const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            select('Codex'); await paint();
            scrollProof = { before, after: pane.scrollTop };
            if (before <= 0 || pane.scrollTop !== 0) throw Error('Switching Agents retained the previous detail scroll: ' + JSON.stringify(scrollProof));
            select('Claude Code'); await paint();
          }
          const rows = [...document.querySelectorAll('.agent-capability-row')], measured = [];
          for (const row of rows) {
            row.scrollIntoView({ block: 'center' }); await new Promise(resolve => requestAnimationFrame(resolve));
            const heading = row.querySelector('h3'), actions = row.querySelector('.agent-capability-actions');
            const title = heading.getBoundingClientRect(), action = actions.getBoundingClientRect();
            measured.push({ label: heading.textContent, overflow: row.scrollWidth > row.clientWidth + 1,
              overlap: title.right > action.left, labelHeight: title.height,
              actionVisible: action.left >= 0 && action.right <= innerWidth + 1 && action.top >= 0 && action.bottom <= innerHeight });
          }
          document.querySelector('.detail-pane').scrollTop = 0;
          await new Promise(resolve => requestAnimationFrame(resolve));
          return { rows: measured, scrollProof, bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
            detailsExpanded: document.querySelector('.native-details').open,
            text: document.querySelector('.agent-detail').innerText };
        })()`);
        if (geometry.bodyOverflow || geometry.detailsExpanded || geometry.rows.some(row => row.overflow || row.overlap || !row.actionVisible || row.labelHeight > 48)) throw Error(`Agent detail geometry: ${JSON.stringify({ agent, language, viewport, geometry })}`);
        const id = `agent-detail-${agent}-${language}-${theme}-${viewport.width}`;
        const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
        await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
        results.push({ id, state: 'green', agent, language, theme, viewport, geometry, screenshot: `${id}.png` });
      }
    }
  }
  return results;
}
