import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const outputRoot = path.resolve(process.argv[4]);
if (!Number.isInteger(port) || !baseUrl || !process.argv[4]) {
  throw new Error('Usage: node session-window-active.mjs <cdp-port> <base-url> <output-directory>');
}

try {
  await mkdir(outputRoot, { recursive: true });
  const client = await connectPage(port);
  try {
    await navigate(client, new URL('session-window.html', baseUrl).href, { width: 1280, height: 900 });
    await waitFor(client, 'Boolean(window.sessionWindow)', { timeout: 7000 });
    const count = await evaluate(client, `import('/session-window-scenarios.mjs').then(module => module.sessionWindowScenarioCount)`);
    if (!Number.isInteger(count) || count < 22) throw new Error('Required session scenarios are missing');
    const batches = Array.from({ length: Math.ceil(count / 4) }, (_, index) => [index * 4, Math.min(count, (index + 1) * 4)]);
    const checks = [];
    for (const [start, end] of batches) {
      const batch = await evaluate(client, `import('/session-window-scenarios.mjs').then(module => module.runSessionWindowScenarios(${start}, ${end}))`);
      checks.push(...batch.results);
      for (const item of batch.results) {
        process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
    }
    const agentLabels = [];
    for (const [width, height] of [[1200, 813], [755, 600]]) for (const language of ['zh', 'en']) {
      await navigate(client, new URL('session-window.html', baseUrl).href, { width, height });
      await waitFor(client, 'Boolean(window.sessionWindow)');
      await evaluate(client, `(() => {
        const control = window.sessionWindow;
        control.reset(); control.language = ${JSON.stringify(language)};
        const session = 'observation-session/dsh-name';
        control.requests[session] = [control.makeRequest(session, 0, Date.now() - 1000)];
        control.views.sessions = query => {
          const page = control.defaultRead('sessions', query);
          return { ...page, sessions: page.sessions.map(row => ({ ...row, agent_id: 'agent_dsh_default' })) };
        };
      })()`);
      await waitFor(client, 'document.querySelector(".session-item")?.textContent.includes("DeepSeek Harness")');
      await evaluate(client, 'document.querySelector(".session-item").click()');
      await waitFor(client, 'document.querySelector(".transcript-head p")?.textContent.includes("DeepSeek Harness")');
      const geometry = await evaluate(client, `(() => {
        const el = document.querySelector('.transcript-head p'), b = el.getBoundingClientRect();
        return { text: el.textContent, left: b.left, right: b.right, width: b.width,
          visible: b.width > 0 && b.height > 0, horizontalOverflow: el.scrollWidth > el.clientWidth + 1,
          outside: b.left < -1 || b.right > innerWidth + 1 };
      })()`);
      const screenshot = `session-agent-dsh-${language}-${width}x${height}.png`;
      const capture = await client.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(path.join(outputRoot, screenshot), Buffer.from(capture.data, 'base64'));
      agentLabels.push({ name: `DSH label ${language} ${width}x${height}`, screenshot, geometry,
        state: geometry.visible && !geometry.horizontalOverflow && !geometry.outside ? 'green' : 'red' });
    }
    const layout = [];
    for (const [width, height] of [[1200, 813], [755, 600]]) {
      for (const language of ['zh', 'en']) {
        for (const state of ['searching', 'results', 'empty', 'pending', 'error']) {
          await navigate(client, new URL('session-window.html', baseUrl).href, { width, height });
          await waitFor(client, 'Boolean(window.sessionWindow)');
          await evaluate(client, `(() => {
            const control = window.sessionWindow;
            control.reset(); control.language = ${JSON.stringify(language)};
            const session = 'observation-session/layout';
            const request = control.makeRequest(session, 0, Date.now() - 1000);
            control.requests[session] = [request];
            control.texts['layout-content'] = '中文 literal checksum matching content';
            const state = ${JSON.stringify(state)};
            control.views.session_search = () => {
              if (state === 'searching') return new Promise(() => {});
              if (state === 'error') throw { code: 'DAEMON_UNAVAILABLE' };
              return { hits: state === 'results' ? [{ session_id: session, request_id: request.request_id, content_id: 'layout-content', original_text_offset: 15 }] : [], next_cursor: state === 'results' ? 'next' : null, index_partial: state === 'pending' };
            };
          })()`);
          await evaluate(client, 'new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
          await waitFor(client, 'Boolean(document.querySelector(".session-filters input"))');
          await evaluate(client, `(() => {
            const input = document.querySelector('.session-filters input');
            Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, 'checksum');
            input.dispatchEvent(new Event('input', { bubbles: true }));
          })()`);
          const condition = state === 'searching' ? 'Boolean(document.querySelector(".session-list [role=status]"))'
            : state === 'error' ? 'Boolean(document.querySelector(".session-master [role=alert]"))'
              : state === 'results' ? 'Boolean(document.querySelector(".session-more"))'
                : 'Boolean(document.querySelector(".session-list-empty"))';
          await waitFor(client, condition);
          const geometry = await evaluate(client, `(() => {
            const elements = [...document.querySelectorAll('.session-filters input,.session-master button,.session-list-empty h3,.session-list-empty p,.session-master [role=alert],.session-list [role=status]')].filter(el => el.getClientRects().length);
            const outside = elements.filter(el => { const b=el.getBoundingClientRect(); return b.left < -1 || b.right > innerWidth+1 || b.top < -1 || b.bottom > innerHeight+1; }).map(el => el.className);
            const clipped = elements.filter(el => {
              const b=el.getBoundingClientRect(); let left=b.left,right=b.right,top=b.top,bottom=b.bottom;
              for (let parent=el.parentElement;parent;parent=parent.parentElement) {
                const style=getComputedStyle(parent),p=parent.getBoundingClientRect();
                if (['hidden','clip','auto','scroll'].includes(style.overflowX)) { left=Math.max(left,p.left);right=Math.min(right,p.right); }
                if (['hidden','clip','auto','scroll'].includes(style.overflowY)) { top=Math.max(top,p.top);bottom=Math.min(bottom,p.bottom); }
              }
              return right-left < b.width-2 || bottom-top < b.height-2;
            }).map(el => el.className);
            return { width: innerWidth, height: innerHeight, outside, clipped, inspected: elements.length, documentOverflow: document.documentElement.scrollWidth > innerWidth+1 };
          })()`);
          const image = await client.send('Page.captureScreenshot', { format: 'png' });
          const file = `session-search-${language}-${width}x${height}-${state}.png`;
          await writeFile(path.join(outputRoot, file), Buffer.from(image.data, 'base64'));
          const result = { name: `session search layout ${language} ${width}x${height} ${state}`, state: !geometry.inspected || geometry.outside.length || geometry.clipped.length || geometry.documentOverflow ? 'red' : 'green', geometry, screenshot: file };
          layout.push(result);
          process.stdout.write(`${result.state}: ${result.name}\n`);
        }
      }
    }
    const report = {
      generatedAt: new Date().toISOString(),
      evidence: 'React components + mock IPC (window and cursor binding mirrored) only; not native/Tauri and not the real daemon',
      checks,
      agentLabels,
      layout,
      summary: { total: checks.length, green: checks.filter(check => check.state === 'green').length, red: checks.filter(check => check.state === 'red').length },
    };
    await writeFile(path.join(outputRoot, 'session-window-report.json'), `${JSON.stringify(report, null, 2)}\n`);
    if (report.summary.total !== count || report.summary.red > 0 || [...layout, ...agentLabels].some(item => item.state !== 'green')) process.exitCode = 1;
  } finally {
    client.close();
  }
} catch (error) {
  await mkdir(outputRoot, { recursive: true });
  await writeFile(path.join(outputRoot, 'session-window-report.json'), `${JSON.stringify({ generatedAt: new Date().toISOString(), harness_error: error instanceof Error ? error.message : String(error) }, null, 2)}\n`);
  throw error;
}
