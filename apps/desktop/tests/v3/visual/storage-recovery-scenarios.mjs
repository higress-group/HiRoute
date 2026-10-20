import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { evaluate, navigate, waitFor } from './cdp-client.mjs';

// Presentation coverage through the real Desktop and bundled Windows page.
// Host replies are fixtures; this does not establish installed recovery acceptance.
export async function runStorageRecoveryScenarios(client, baseUrl, output) {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const results = [];
  const cases = [
    { cause: 'missing_package', action: 'preserve_and_diagnose' },
    { cause: 'insufficient_space', action: 'retry_current', storage_space: { required_bytes: 1073741824, available_bytes: 1048576 } },
    { cause: 'namespace_unavailable', action: 'preserve_and_diagnose' },
    { cause: 'migration_failed', action: 'resume_current', backup_batch: 'set_original', backup_name: 'migration-set' },
    { cause: 'migration_failed', action: 'restore_complete_group', backup_batch: 'set_original', backup_name: 'migration-set' },
    { variant:'long-location', cause:'migration_failed',action:'restore_complete_group',backup_batch:'set_'+ 'a'.repeat(100),backup_name:'observation-format-5.'+'b'.repeat(100) },
    { cause: 'future_cause', action: 'future_action' },
  ];
  for (const platform of ['macos', 'windows']) for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) for (const [width, height] of [[1200, 813], [755, 600]]) for (const fixture of cases) {
    const {variant,...facts}=fixture;
    const report = { schema: 'hiroute.storage-startup-recovery/v1', ...facts };
    const hasSource = ['resume_current', 'restore_complete_group'].includes(fixture.action);
    const id = `${platform}.${fixture.cause}.${fixture.action}.${language}.${theme}.${width}${variant?'.'+variant:''}`;
    await client.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    let script;
    try {
      if (platform === 'macos') {
        await navigate(client, new URL('product-shell.html', baseUrl).href, { width, height });
        await waitFor(client, 'Boolean(window.productShell)');
        await evaluate(client, `(()=>{
          localStorage.setItem('hiroute.language', ${JSON.stringify(language)});
          localStorage.setItem('hiroute.theme', ${JSON.stringify(theme)});
          window.productShell.handlers.desktop_snapshot=()=>{throw 'DAEMON_NOT_READY';};
          window.productShell.handlers.startup_status=()=>({state:'failed',code:'DAEMON_UPGRADE_STORAGE_FAILED',recovery_available:${hasSource},backup_directory:${JSON.stringify(hasSource ? `/Users/example/Library/Application Support/ai.hiroute.desktop/storage.upgrade-backups/${fixture.backup_name}/source` : undefined)},storage_recovery:${JSON.stringify(report)}});
          window.productShell.remount();
        })()`);
        await waitFor(client, 'Boolean(document.querySelector("[data-error-code=DAEMON_UPGRADE_STORAGE_FAILED]"))');
      } else {
        script = await client.send('Page.addScriptToEvaluateOnNewDocument', { source: `Object.defineProperty(navigator,'language',{value:${JSON.stringify(language)}});window.__TAURI_INTERNALS__={invoke:async()=>({phase:'failed',code:'DAEMON_UPGRADE_STORAGE_FAILED',logs_path:'C:\\\\Users\\\\example\\\\AppData\\\\Local\\\\HiRoute\\\\diagnostics',storage_recovery:${JSON.stringify(report)}})};` });
        const shell = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../shell/index.html');
        await navigate(client, new URL('/@fs' + shell, baseUrl).href, { width, height });
        await waitFor(client, 'document.querySelector("#failure")?.textContent === "DAEMON_UPGRADE_STORAGE_FAILED"');
      }
      const selector = platform === 'macos' ? '[data-error-code=DAEMON_UPGRADE_STORAGE_FAILED]' : '#explanation';
      const geometry = await evaluate(client, `(()=>{const e=document.querySelector(${JSON.stringify(selector)}); e.scrollIntoView({block:'start'});const b=e.getBoundingClientRect();return{text:e.textContent,left:b.left,right:b.right,overflow:e.scrollWidth>e.clientWidth+1,height:b.height,bodyOverflow:document.documentElement.scrollWidth>innerWidth+1,retry:!!document.querySelector('#retry')&&document.querySelector('#retry').getClientRects().length>0};})()`);
      if (platform === 'macos') assert(geometry.height < height * 0.8, `Startup guidance stretched to fill the page: ${id}`);
      assert(!geometry.overflow && !geometry.bodyOverflow && geometry.left >= 0 && geometry.right <= width + 1, `Recovery guidance overflow: ${id} ${JSON.stringify(geometry)}`);
      assert((language === 'zh' ? /保留|另存/ : /keep|preserv/i).test(geometry.text), `Preservation guidance missing: ${id}`);
      if (fixture.action === 'preserve_and_diagnose' || fixture.action === 'future_action') {
        assert(!geometry.text.includes(language === 'zh' ? '尚未改变原数据组' : 'has not changed the original data group'), `Unproven original compatibility: ${id}`);
      }
      const expected = {
        missing_package: language === 'zh' ? '配套旧版本' : 'matching complete previous package',
        insufficient_space: 'MiB',
        namespace_unavailable: language === 'zh' ? '同一文件系统' : 'same filesystem',
        migration_failed: language === 'zh' ? '批次' : 'batch',
        future_cause: language === 'zh' ? '无法确认' : 'could not be established',
      }[fixture.cause];
      assert(geometry.text.includes(expected), `Specific reason/action missing: ${id}`);
      const located=await evaluate(client,`(()=>{const e=document.querySelector(${JSON.stringify(platform==='macos'?'.recovery-location':'#recovery-material')});return e&&e.getClientRects().length?{text:e.textContent,overflow:e.scrollWidth>e.clientWidth+1}:null;})()`);
      if(hasSource){
        assert(located&&!located.overflow&&located.text.includes(fixture.backup_batch)&&located.text.includes(fixture.backup_name)&&located.text.includes('RECOVERY.md'),`Original recovery location missing or overflowing: ${id}`);
        if(platform==='macos') assert(geometry.text.includes(language==='zh'?'查看保留的恢复材料与说明':'Inspect preserved recovery material and guide'),`Recovery directory action missing: ${id}`);
      }else assert(!located,`Unverified recovery location exposed: ${id}`);
      if (platform === 'windows') assert(geometry.retry === ['retry_current', 'resume_current'].includes(fixture.action), `Unsafe or missing retry: ${id}`);
      const screenshot = `${id}.png`;
      const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      await writeFile(path.join(output, screenshot), Buffer.from(shot.data, 'base64'));
      results.push({ id, state: 'green', screenshot, geometry });
    } catch (error) {
      await writeFile(path.join(output, id + '.failure.json'), JSON.stringify({error:String(error),page:await evaluate(client, '({url:location.href,state:document.readyState,text:document.body.innerText,resources:performance.getEntriesByType("resource").map(r=>({name:r.name,status:r.responseStatus,duration:r.duration}))})')},null,2));
      throw error;
    } finally {
      if (script) await client.send('Page.removeScriptToEvaluateOnNewDocument', { identifier: script.identifier });
    }
  }
  await writeFile(path.join(output, 'storage-recovery-report.json'), JSON.stringify({ evidence: 'Production Desktop and Windows bundled HTML/styles with synthetic startup replies. Native recovery remains separate.', results }, null, 2) + '\n');
  process.stdout.write(`green: ${results.length} storage recovery presentation states\n`);
}
