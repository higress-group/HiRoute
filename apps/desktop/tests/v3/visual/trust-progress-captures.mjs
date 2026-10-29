import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

export async function trustProgressCaptures(client, baseUrl, output) {
  const results = [];
  for (const state of ['pending', 'error']) for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
    await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
    await waitFor(client, 'Boolean(window.productShell)');
    await evaluate(client, `(() => {
      localStorage.setItem('hiroute.language', ${JSON.stringify(language)});
      localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)});
      const c = window.productShell;
      c.handlers.observation_read = payload => {
        if (payload.request.intent.view !== 'plan_trust_suggestions') return c.fixtureResponse('observation_read', payload);
        if (${JSON.stringify(state)} === 'error') throw { code: 'OBSERVATION_UNAVAILABLE' };
        return { suggestions: [], unavailable: [], pending: ['plan/daily'] };
      };
      c.remount();
    })()`);
    const selector = `.trust-attention [role="${state === 'pending' ? 'status' : 'alert'}"]`;
    await waitFor(client, `Boolean(document.querySelector(${JSON.stringify(selector)}))`);
    const geometry = await evaluate(client, `(() => {
      const box = document.querySelector('.trust-attention'); box.scrollIntoView({ block: 'center' });
      const row = box.querySelector(${JSON.stringify(`[role="${state === 'pending' ? 'status' : 'alert'}"]`)});
      const b = box.getBoundingClientRect(), r = row.getBoundingClientRect();
      return { bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
        overflow: box.scrollWidth > box.clientWidth + 1 || row.scrollWidth > row.clientWidth + 1,
        visible: r.left >= 0 && r.right <= innerWidth + 1 && r.top >= 0 && r.bottom <= innerHeight + 1,
        height: b.height, text: row.textContent, color: getComputedStyle(row).color };
    })()`);
    if (geometry.bodyOverflow || geometry.overflow || !geometry.visible) throw new Error(`Trust progress geometry: ${JSON.stringify({ state, language, theme, viewport, geometry })}`);
    const id = `trust-progress-${state}-${language}-${theme}-${viewport.width}`;
    const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
    results.push({ id, state: 'green', readState: state, language, theme, viewport, geometry, screenshot: `${id}.png` });
  }
  return results;
}
