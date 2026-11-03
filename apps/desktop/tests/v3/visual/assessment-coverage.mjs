import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const outputRoot = path.resolve(process.argv[4]);
if (!Number.isInteger(port) || !baseUrl || !process.argv[4]) {
  throw new Error('Usage: assessment-coverage.mjs <cdp-port> <base-url> <output-directory>');
}

await mkdir(outputRoot, { recursive: true });
const client = await connectPage(port);
const checks = [];
const captures = [];
try {
  for (const [width, height] of [[1200, 813], [755, 600]]) {
    for (const language of ['zh', 'en']) for (const theme of ['light', 'dark']) {
      await navigate(client, new URL('session-window.html', baseUrl).href, { width, height });
      await waitFor(client, 'Boolean(window.sessionWindow)');
      const result = await evaluate(client, `import('/session-window-scenarios.mjs').then(module => module.runAssessmentCoverageScenario())`);
      checks.push({ width, height, language, theme, ...result });
      if (result.failed) console.log(JSON.stringify({ width, height, language, theme, results: result.results }));
      await evaluate(client, `(() => {
        const control = window.sessionWindow;
        control.language = ${JSON.stringify(language)}; control.theme = ${JSON.stringify(theme)};
        control.bumpRefresh();
      })()`);
      await evaluate(client, 'new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
      await waitFor(client, `document.querySelector('.quality-refresh')?.textContent.includes(${JSON.stringify(language === 'zh' ? '刷新' : 'Refresh')}) && !document.querySelector('.quality-refresh').disabled && document.querySelectorAll('.quality-row').length === 4`);
      for (const expanded of [false, true]) {
        await evaluate(client, `(() => {
          const row = document.querySelector('[data-stage-id="stage/coverage/partial"]');
          const details = row.querySelector('.quality-assessment-details');
          if (details.open !== ${expanded}) details.querySelector('summary').click();
          row.scrollIntoView({ block: 'start', inline: 'nearest' });
        })()`);
        const geometry = await evaluate(client, `(() => {
          const row = document.querySelector('[data-stage-id="stage/coverage/partial"]');
          const r = row.getBoundingClientRect(), main = row.querySelector('.quality-row-main').getBoundingClientRect(), score = row.querySelector('.quality-score').getBoundingClientRect();
          return { visible: r.width > 0 && r.height > 0, horizontalOverflow: row.scrollWidth > row.clientWidth + 1,
            headerOverlap: main.right > score.left + 1, outside: r.left < -1 || r.right > innerWidth + 1,
            text: row.innerText, expanded: row.querySelector('.quality-assessment-details').open };
        })()`);
        const screenshot = `assessment-coverage-${language}-${theme}-${width}x${height}-${expanded ? 'expanded' : 'collapsed'}.png`;
        const capture = await client.send('Page.captureScreenshot', { format: 'png' });
        await writeFile(path.join(outputRoot, screenshot), Buffer.from(capture.data, 'base64'));
        captures.push({ screenshot, width, height, language, theme, geometry,
          state: geometry.visible && !geometry.horizontalOverflow && !geometry.headerOverlap && !geometry.outside && geometry.expanded === expanded ? 'green' : 'red' });
      }
    }
  }
  const failed = checks.some(check => check.failed) || captures.some(capture => capture.state !== 'green');
  await writeFile(path.join(outputRoot, 'assessment-coverage-results.json'), JSON.stringify({
    evidence: 'Production Sessions/PlanQuality components and styles with controlled IPC; native acceptance is separate.',
    state: failed ? 'red' : 'green', checks, captures,
  }, null, 2));
  console.log(JSON.stringify({ state: failed ? 'red' : 'green', combinations: checks.length,
    scenarios_failed: checks.filter(check => check.failed).map(check => check.results),
    capture_failures: captures.filter(capture => capture.state !== 'green').map(capture => capture.screenshot), screenshots: captures.length }));
  if (failed) process.exitCode = 1;
} finally {
  client.close();
}
