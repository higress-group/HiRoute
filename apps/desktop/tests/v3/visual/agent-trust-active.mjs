import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const outputRoot = path.resolve(process.argv[4]);
const routeSaveOnly = process.argv[5] === '--route-save-only';
const workerReplacementOnly = process.argv[5] === '--worker-replacement-only';
const claudeCollaborationOnly = process.argv[5] === '--claude-collaboration-only';
if (!Number.isInteger(port) || !baseUrl || !process.argv[4]) {
  throw new Error('Usage: node agent-trust-active.mjs <cdp-port> <base-url> <output-directory>');
}

try {
  await mkdir(outputRoot, { recursive: true });
  const client = await connectPage(port);
  try {
    const checks = [];
    if (!routeSaveOnly && !workerReplacementOnly) {
      await navigate(client, new URL('agent-trust.html', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.agentTrust)', { timeout: 7000 });
      const batches = claudeCollaborationOnly ? [[11, 14]] : [[0, 5], [5, 10], [10, 17]];
      for (const [start, end] of batches) {
        const batch = await evaluate(client, `import('/agent-trust-scenarios.mjs').then(module => module.runAgentTrustScenarios(${start}, ${end}))`);
        checks.push(...batch.results);
        for (const item of batch.results) {
          process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
        }
      }
    }
    if (!claudeCollaborationOnly) {
      await navigate(client, new URL('?page=routing&scenario=ready', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.__HIRouteFixtureTrace)', { timeout: 7000 });
      const routing = await evaluate(client, routeSaveOnly
        ? "import('/routing-worker-scenarios.mjs').then(module => module.runRoutingWorkerScenarios(5, 6))"
        : workerReplacementOnly
          ? "import('/routing-worker-scenarios.mjs').then(module => module.runRoutingWorkerScenarios(3, 5))"
          : "import('/routing-worker-scenarios.mjs').then(module => module.runRoutingWorkerScenarios())");
      checks.push(...routing.results);
      for (const item of routing.results) {
        process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
    }
    if (!routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly) {
      await navigate(client, new URL('?page=models&scenario=ready', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.__HIRouteFixtureTrace)', { timeout: 7000 });
      const fromModel = await evaluate(client, "import('/model-to-route-scenarios.mjs').then(module => module.runModelToRouteScenarios())");
      checks.push(...fromModel.results);
      for (const item of fromModel.results) {
        process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
    }
    const report = {
      generatedAt: new Date().toISOString(),
      evidence: 'React components + mock IPC only; not native/Tauri and not the real daemon',
      checks,
      summary: { total: checks.length, green: checks.filter(check => check.state === 'green').length, red: checks.filter(check => check.state === 'red').length },
    };
    await writeFile(path.join(outputRoot, 'agent-trust-report.json'), `${JSON.stringify(report, null, 2)}\n`);
    if (report.summary.red > 0) process.exitCode = 1;
  } finally {
    client.close();
  }
} catch (error) {
  await mkdir(outputRoot, { recursive: true });
  await writeFile(path.join(outputRoot, 'agent-trust-report.json'), `${JSON.stringify({ generatedAt: new Date().toISOString(), harness_error: error instanceof Error ? error.message : String(error) }, null, 2)}\n`);
  throw error;
}
