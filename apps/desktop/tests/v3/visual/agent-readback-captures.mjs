import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Production components/styles, controlled IPC. No native save or latency claim.
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
