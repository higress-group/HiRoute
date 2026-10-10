import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { selectScenarios, assertScenarioResults } from '../browser/scenario-selection.mjs';
import { agentTrustRequired, routingWorkerRequired, productShellRequired, focusedRequirements } from '../browser/scenario-requirements.mjs';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const outputRoot = path.resolve(process.argv[4]);
const shellOnly = process.argv[5] === '--shell-only';
const routeSaveOnly = process.argv[5] === '--route-save-only';
const workerReplacementOnly = process.argv[5] === '--worker-replacement-only';
const claudeCollaborationOnly = process.argv[5] === '--claude-collaboration-only';
const routeOptionsOnly = process.argv[5] === '--route-options-only';
if (!Number.isInteger(port) || !baseUrl || !process.argv[4]) {
  throw new Error('Usage: node agent-trust-active.mjs <cdp-port> <base-url> <output-directory>');
}

try {
  await mkdir(outputRoot, { recursive: true });
  const client = await connectPage(port);
  try {
    const checks = [];
    if (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !routeOptionsOnly) {
      await navigate(client, new URL('agent-trust.html', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.agentTrust)', { timeout: 7000 });
      const catalog = await evaluate(client, "import('/agent-trust-scenarios.mjs').then(module => module.agentTrustScenarioCatalog)");
      const selected = selectScenarios(catalog, claudeCollaborationOnly
        ? { capability: 'claude-collaboration', requiredIds: focusedRequirements['claude-collaboration'] }
        : { requiredIds: agentTrustRequired });
      const results = [];
      for (let offset = 0; offset < selected.length; offset += 5) {
        const ids = selected.slice(offset, offset + 5).map(item => item.id);
        const batch = await evaluate(client, `import('/agent-trust-scenarios.mjs').then(module => module.runAgentTrustScenarios(${JSON.stringify({ ids, requiredIds: ids })}))`);
        results.push(...batch.results);
        for (const item of batch.results) {
          process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.id} · ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
        }
      }
      assertScenarioResults(selected, results);
      checks.push(...results);
    }
    if (!shellOnly && !claudeCollaborationOnly && !routeOptionsOnly) {
      await navigate(client, new URL('?page=routing&scenario=ready', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.__HIRouteFixtureTrace)', { timeout: 7000 });
      const catalog = await evaluate(client, "import('/routing-worker-scenarios.mjs').then(module => module.routingWorkerScenarioCatalog)");
      const capability = routeSaveOnly ? 'route-save' : workerReplacementOnly ? 'worker-replacement' : undefined;
      const selection = { capability, requiredIds: capability ? focusedRequirements[capability] : routingWorkerRequired };
      const selected = selectScenarios(catalog, selection);
      const routing = await evaluate(client, `import('/routing-worker-scenarios.mjs').then(module => module.runRoutingWorkerScenarios(${JSON.stringify(selection)}))`);
      assertScenarioResults(selected, routing.results);
      checks.push(...routing.results);
      for (const item of routing.results) {
        process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
    }
    if (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly && !routeOptionsOnly) {
      await navigate(client, new URL('?page=models&scenario=ready', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.__HIRouteFixtureTrace)', { timeout: 7000 });
      const fromModel = await evaluate(client, "import('/model-to-route-scenarios.mjs').then(module => module.runModelToRouteScenarios())");
      if (fromModel.tests !== 1 || fromModel.results?.length !== 1 || !['green', 'red'].includes(fromModel.results[0].state)) {
        throw new Error('Required model-to-route scenario did not execute');
      }
      checks.push(...fromModel.results);
      for (const item of fromModel.results) {
        process.stdout.write(`${item.state === 'green' ? 'green' : 'red'}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
    }
    if (routeOptionsOnly || (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly)) {
      await navigate(client, new URL('plan-editor-ux.html?checks=availability', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(document.querySelector("button")?.textContent?.includes("运行智能路由交互检查"))', { timeout: 7000 });
      await evaluate(client, 'document.querySelector("button").click()');
      await waitFor(client, 'Boolean(document.querySelector("[data-ux-results]")?.textContent)', { timeout: 20000 });
      const result = await evaluate(client, 'JSON.parse(document.querySelector("[data-ux-results]").textContent)');
      const required = ['routing.options.id-safety', 'routing.options.exclusions', 'routing.options.service-error', 'routing.options.empty'];
      if (result.tests !== required.length || result.results?.length !== required.length
        || result.results.some(item => !['green', 'red'].includes(item.state))
        || new Set(result.results.map(item => item.name.split(':')[0])).size !== required.length
        || required.some(id => !result.results.some(item => item.name.startsWith(`${id}:`)))) {
        throw new Error('Required route-options scenarios did not execute');
      }
      checks.push(...result.results);
      for (const item of result.results) process.stdout.write(`${item.state}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
    }
    if (shellOnly || (!routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly && !routeOptionsOnly)) {
      await navigate(client, new URL('product-shell.html', baseUrl).href, { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.productShell)', { timeout: 7000 });
      const catalog = await evaluate(client, "import('/product-shell-scenarios.mjs').then(module => module.productShellScenarioCatalog)");
      const selected = selectScenarios(catalog, { requiredIds: productShellRequired });
      const results = [];
      for (const { id } of selected) {
        const batch = await evaluate(client, `import('/product-shell-scenarios.mjs').then(module => module.runProductShellScenarios(${JSON.stringify({ ids: [id], requiredIds: [id] })}))`);
        results.push(...batch.results);
        for (const item of batch.results) process.stdout.write(`${item.state}: ${item.id} · ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
      assertScenarioResults(selected, results);
      checks.push(...results);
    }
    if (checks.length === 0) throw new Error('No scenarios executed');
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
