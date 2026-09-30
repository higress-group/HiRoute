import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const outputRoot = process.argv[4] && path.resolve(process.argv[4]);
if (!Number.isInteger(port) || !baseUrl || !outputRoot) {
  throw new Error('Usage: node subscription-repair-active.mjs <cdp-port> <base-url> <output-directory>');
}
const client = await connectPage(port);
try {
  await navigate(client, new URL('subscription-repair.html', baseUrl).href, { width: 1280, height: 900 });
  await waitFor(client, 'Boolean(window.subscriptionRepair)', { timeout: 7000 });
  const report = await evaluate(client, "import('/subscription-repair-scenarios.mjs').then(module => module.runSubscriptionRepairScenarios())");
  for (const item of report.results) {
    process.stdout.write(`${item.state}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
  }
  await mkdir(outputRoot, { recursive: true });
  await writeFile(path.join(outputRoot, 'subscription-repair-report.json'), `${JSON.stringify(report, null, 2)}\n`);
  if (report.tests !== 15 || report.failed > 0) process.exitCode = 1;
} finally {
  client.close();
}
