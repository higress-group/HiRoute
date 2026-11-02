import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { evaluate, waitFor } from './cdp-client.mjs';
export async function runStorageCapacityCaptures(client, output) {
  const checks = [];
  for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200, 813], [755, 600]]) {
    await client.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
    await evaluate(client, `window.productShell.reset(); localStorage.setItem('hiroute.language',${JSON.stringify(language)}); localStorage.setItem('hiroute.theme',${JSON.stringify(theme)}); window.productShell.remount()`);
    const label = language === 'zh' ? '设置' : 'Settings';
    await waitFor(client, `Boolean(document.querySelector('button[aria-label="${label}"]'))`);
    await evaluate(client, `window.productShell.handlers.observation_capacity_get=()=>({max_active_bytes:20_000_000_000,active_bytes:21_000_000_000,measured_at_ms:1000,state:'pending',reason:'physical_release'});document.querySelector('button[aria-label="${label}"]').click()`);
    await waitFor(client, `Boolean(document.querySelector('.settings-tabs [role="tab"]'))`);
    await evaluate(client, `Array.from(document.querySelectorAll('.settings-tabs button')).find(button=>button.textContent===${JSON.stringify(language === 'zh' ? '记录与诊断' : 'Records & diagnostics')}).click()`);
    const field = `document.querySelector('input[aria-label="${language === 'zh' ? '记录容量上限（GB）' : 'Record storage limit (GB)'}"]')`;
    await waitFor(client, `Boolean(${field} && !${field}.disabled && ${field}.value==='20')`);
    await evaluate(client, `${field}.scrollIntoView({block:'center'})`);
    const geometry = await evaluate(client, `(()=>{const field=${field},row=field.closest('.settings-row'),copy=row.querySelector('.settings-copy'),button=row.querySelector('button');return{overflow:row.scrollWidth>row.clientWidth+1,copy:copy.getBoundingClientRect().width,controls:[field,button].map(e=>{const b=e.getBoundingClientRect();return{left:b.left,right:b.right,top:b.top,bottom:b.bottom}})}})()`);
    if (geometry.overflow || geometry.copy < 160 || geometry.controls.some(b => b.left < 0 || b.right > width + 1 || b.top < 0 || b.bottom > height + 1)) throw new Error('Storage settings geometry failed: ' + JSON.stringify(geometry));
    const screenshot = `storage-capacity-${language}-${theme}-${width}x${height}.png`;
    const shot = await client.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(output, screenshot), Buffer.from(shot.data, 'base64'));
    checks.push({ id: `desktop.settings.storage-capacity.${language}.${theme}.${width}`, state: 'green', screenshot, geometry });
  }
  return checks;
}
