import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

export async function nativeConnectionCaptures(client, baseUrl, output) {
  const results = [];
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const viewport of [{ width: 1200, height: 813 }, { width: 755, height: 600 }]) {
    await navigate(client, new URL('product-shell.html', baseUrl).href, viewport);
    await waitFor(client, 'Boolean(window.productShell)');
    await evaluate(client, `(() => {
      localStorage.setItem('hiroute.language', ${JSON.stringify(language)});
      localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)});
      const c = window.productShell;
      c.desktop.native_connection = 'recovery_required'; c.desktop.trusted_authority = false;
      c.handlers.observation_read = () => { throw { source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' }; };
      c.handlers.recover_native_connection = () => { throw { source: 'native', code: 'AUTHORITY_RECOVERY_WORK_ACTIVE' }; };
      c.remount();
    })()`);
    await waitFor(client, 'Boolean(document.querySelector(".native-connection-recovery button"))');
    await evaluate(client, 'document.querySelector(".native-connection-recovery button").click()');
    await waitFor(client, 'Boolean(document.querySelector("[data-error-code=AUTHORITY_RECOVERY_WORK_ACTIVE]"))');
    const geometry = await evaluate(client, `(() => {
      const box = document.querySelector('.native-connection-recovery'); const action = box.querySelector('button');
      const b = box.getBoundingClientRect(), a = action.getBoundingClientRect();
      return { bodyOverflow: document.documentElement.scrollWidth > innerWidth + 1,
        overflow: box.scrollWidth > box.clientWidth + 1, height: b.height,
        actionVisible: a.left >= 0 && a.right <= innerWidth + 1 && a.top >= 0 && a.bottom <= innerHeight,
        text: box.textContent };
    })()`);
    if (geometry.overflow || geometry.bodyOverflow || !geometry.actionVisible || geometry.height > viewport.height * .65) throw new Error(`Recovery geometry: ${JSON.stringify({ language, theme, viewport, geometry })}`);
    const id = `native-connection-${language}-${theme}-${viewport.width}`;
    const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    await writeFile(path.join(output, `${id}.png`), Buffer.from(shot.data, 'base64'));
    results.push({ id, state: 'green', language, theme, viewport, geometry, screenshot: `${id}.png` });
  }
  return results;
}
