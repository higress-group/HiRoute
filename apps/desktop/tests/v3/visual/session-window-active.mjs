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
    const report = {
      generatedAt: new Date().toISOString(),
      evidence: 'React components + mock IPC (window and cursor binding mirrored) only; not native/Tauri and not the real daemon',
      checks,
      summary: { total: checks.length, green: checks.filter(check => check.state === 'green').length, red: checks.filter(check => check.state === 'red').length },
    };
    await writeFile(path.join(outputRoot, 'session-window-report.json'), `${JSON.stringify(report, null, 2)}\n`);
    if (report.summary.total !== count || report.summary.red > 0) process.exitCode = 1;
  } finally {
    client.close();
  }
} catch (error) {
  await mkdir(outputRoot, { recursive: true });
  await writeFile(path.join(outputRoot, 'session-window-report.json'), `${JSON.stringify({ generatedAt: new Date().toISOString(), harness_error: error instanceof Error ? error.message : String(error) }, null, 2)}\n`);
  throw error;
}
