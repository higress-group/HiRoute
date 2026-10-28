import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { selectScenarios, assertScenarioResults } from '../browser/scenario-selection.mjs';
import { agentTrustRequired, routingWorkerRequired, productShellRequired, focusedRequirements } from '../browser/scenario-requirements.mjs';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const hostTransport = process.env.HIROUTE_BROWSER_TRANSPORT ?? 'tauri';
if (!['tauri', 'console'].includes(hostTransport)) throw new Error('Unknown browser host transport');
const fixtureUrl = relative => { const url = new URL(relative, baseUrl); if (hostTransport === 'console') url.searchParams.set('transport', 'console'); return url.href; };

const outputRoot = path.resolve(process.argv[4]);
const subscriptionProxyOnly = process.argv[5] === '--subscription-proxy-only';
const agentsOnly = process.argv[5] === '--agents-only';
const shellOnly = process.argv[5] === '--shell-only' || subscriptionProxyOnly;
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
    if (process.argv[5] === '--storage-recovery-only') {
      await (await import('./storage-recovery-scenarios.mjs')).runStorageRecoveryScenarios(client, baseUrl, outputRoot);
    } else if (process.argv[5] === '--shell-host-only') {
      await (await import('./shell-host-scenarios.mjs')).runShellHostScenarios(client, baseUrl, outputRoot);
    } else if (process.argv[5] === '--console-only') {
      await (await import('./console-scenarios.mjs')).runConsoleScenarios(client, baseUrl, outputRoot);
      process.exitCode = 0;
    } else {
    const checks = [];
    if (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !routeOptionsOnly) {
      await navigate(client, fixtureUrl('agent-trust.html'), { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.agentTrust)', { timeout: 7000 });
      const catalog = await evaluate(client, "import('/agent-trust-scenarios.mjs').then(module => module.agentTrustScenarioCatalog)");
      const selected = selectScenarios(catalog, claudeCollaborationOnly
        ? { capability: 'claude-collaboration', requiredIds: focusedRequirements['claude-collaboration'] }
        : { ids: agentsOnly ? process.env.HIROUTE_BROWSER_SCENARIOS?.split(',') : undefined, requiredIds: agentsOnly && process.env.HIROUTE_BROWSER_SCENARIOS ? process.env.HIROUTE_BROWSER_SCENARIOS.split(',') : agentTrustRequired });
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
      if (selected.some(item => item.capabilities.includes('dsh-desktop'))) {
        checks.push(...await (await import('./agent-readback-captures.mjs')).dshDesktopCaptures(client, baseUrl, outputRoot));
      }
      if (selected.some(item => item.capabilities.includes('agent-readback'))) {
        checks.push(...await (await import('./agent-readback-captures.mjs')).agentReadbackCaptures(client, baseUrl, outputRoot));
      }
    }
    if (!agentsOnly && !shellOnly && !claudeCollaborationOnly && !routeOptionsOnly) {
      await navigate(client, fixtureUrl('?page=routing&scenario=ready'), { width: 1280, height: 900 });
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
    if (!agentsOnly && !shellOnly && !routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly && !routeOptionsOnly) {
      await navigate(client, fixtureUrl('?page=models&scenario=ready'), { width: 1280, height: 900 });
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
    if (!agentsOnly && (routeOptionsOnly || (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly))) {
      await navigate(client, fixtureUrl('plan-editor-ux.html?checks=availability'), { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(document.querySelector("button")?.textContent?.includes("运行智能路由交互检查"))', { timeout: 7000 });
      await evaluate(client, 'document.querySelector("button").click()');
      await waitFor(client, 'Boolean(document.querySelector("[data-ux-results]")?.textContent)', { timeout: 20000 });
      const result = await evaluate(client, 'JSON.parse(document.querySelector("[data-ux-results]").textContent)');
      const required = ['routing.options.loading', 'routing.options.id-safety', 'routing.options.exclusions', 'routing.options.service-error', 'routing.options.empty'];
      if (result.tests !== required.length || result.results?.length !== required.length
        || result.results.some(item => !['green', 'red'].includes(item.state))
        || new Set(result.results.map(item => item.name.split(':')[0])).size !== required.length
        || required.some(id => !result.results.some(item => item.name.startsWith(`${id}:`)))) {
        throw new Error('Required route-options scenarios did not execute');
      }
      checks.push(...result.results);
      for (const item of result.results) process.stdout.write(`${item.state}: ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
    }
    if (!agentsOnly && (routeOptionsOnly || (!shellOnly && !routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly))) {
      const required = ['routing.authoring.loading', 'routing.authoring.failure', 'routing.authoring.reopen', 'routing.authoring.dirty'];
      for (const language of ['zh', 'en']) {
        for (const viewport of [{ width: 755, height: 600 }, { width: 1200, height: 813 }]) {
          await navigate(client, new URL(`plan-editor-ux.html?checks=authoring&language=${language}`, baseUrl).href, viewport);
          await waitFor(client, 'Boolean(window.planAuthoringChecks)', { timeout: 7000 });
          const names = await evaluate(client, 'window.planAuthoringChecks.names');
          if (names.length !== required.length || required.some((id, i) => !names[i].startsWith(`${id}:`))) throw new Error('Required authoring cases are missing');
          for (let i = 0; i < required.length; i++) {
            const result = await evaluate(client, `window.planAuthoringChecks.run(${i})`);
            const file = `${required[i]}-${language}-${viewport.width}.png`;
            const shot = await client.send('Page.captureScreenshot', { format: 'png' });
            await writeFile(path.join(outputRoot, file), Buffer.from(shot.data, 'base64'));
            checks.push({ ...result, language, viewport, screenshot: file });
            process.stdout.write(`${result.state}: ${result.name} (${language}, ${viewport.width})${result.error ? ` · ${result.error}` : ''}\n`);
            if (result.state !== 'green') break;
          }
        }
      }
      // Geometry is covered above; sample contrast with the same production states in dark mode.
      await navigate(client, new URL('plan-editor-ux.html?checks=authoring&language=zh&theme=dark', baseUrl).href, { width: 755, height: 600 });
      await waitFor(client, 'Boolean(window.planAuthoringChecks)', { timeout: 7000 });
      for (const i of [1, 2]) {
        const result = await evaluate(client, `window.planAuthoringChecks.run(${i})`);
        const file = `${required[i]}-zh-755-dark.png`;
        const shot = await client.send('Page.captureScreenshot', { format: 'png' });
        await writeFile(path.join(outputRoot, file), Buffer.from(shot.data, 'base64'));
        checks.push({ ...result, language: 'zh', theme: 'dark', viewport: { width: 755, height: 600 }, screenshot: file });
        process.stdout.write(`${result.state}: ${result.name} (dark)${result.error ? ` · ${result.error}` : ''}\n`);
      }
    }
    if (!agentsOnly && (shellOnly || (!routeSaveOnly && !workerReplacementOnly && !claudeCollaborationOnly && !routeOptionsOnly))) {
      await navigate(client, fixtureUrl('product-shell.html'), { width: 1280, height: 900 });
      await waitFor(client, 'Boolean(window.productShell)', { timeout: 7000 });
      const catalog = await evaluate(client, "import('/product-shell-scenarios.mjs').then(module => module.productShellScenarioCatalog)");
      const requested = process.env.HIROUTE_BROWSER_SCENARIOS?.split(',');
      const applicable = hostTransport === 'console' ? catalog.filter(item => !item.capabilities.includes('native-recovery')) : catalog;
      const required = productShellRequired.filter(id => applicable.some(item => item.id === id));
      const selected = selectScenarios(applicable, subscriptionProxyOnly
        ? { capability: 'subscription-proxy', requiredIds: focusedRequirements['subscription-proxy'] }
        : { ids: requested, requiredIds: requested ?? required });
      const results = [];
      for (const { id } of selected) {
        const viewport = ['desktop.models.subscription-scan.recommended-login',
          'desktop.models.view-switch', 'desktop.presentation.control-layout', 'desktop.settings.storage-capacity',
          'desktop.routing.additional-budget-consumers',
          'desktop.models.subscription-login.callback-modes'].includes(id)
          ? { width: 755, height: 600 } : { width: 1280, height: 900 };
        await client.send('Emulation.setDeviceMetricsOverride', { ...viewport, deviceScaleFactor: 1, mobile: false });
        const batch = await evaluate(client, `import('/product-shell-scenarios.mjs').then(module => module.runProductShellScenarios(${JSON.stringify({ ids: [id], requiredIds: [id] })}))`);
        results.push(...batch.results.map(item => ({ ...item, viewport })));
        for (const item of batch.results) process.stdout.write(`${item.state}: ${item.id} · ${item.name}${item.error ? ` · ${item.error}` : ''}\n`);
      }
      assertScenarioResults(selected, results);
      checks.push(...results);
      if (selected.some(s => s.id === 'desktop.home.saved-settings-are-not-verified')) {
        checks.push(...await (await import('./home-summary-captures.mjs')).homeSummaryCaptures(client, baseUrl, outputRoot));
      }
      if (selected.some(s => s.capabilities.includes('home-trust-progress'))) {
        checks.push(...await (await import('./trust-progress-captures.mjs')).trustProgressCaptures(client, baseUrl, outputRoot));
      }
      if (selected.some(s => s.capabilities.includes('native-recovery'))) {
        checks.push(...await (await import('./native-connection-captures.mjs')).nativeConnectionCaptures(client, baseUrl, outputRoot));
      }
      if (selected.some(s => s.id === 'desktop.settings.storage-capacity')) {
        checks.push(...await (await import('./storage-capacity-captures.mjs')).runStorageCapacityCaptures(client, outputRoot));
      }
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
    }
  } finally {
    client.close();
  }
} catch (error) {
  await mkdir(outputRoot, { recursive: true });
  await writeFile(path.join(outputRoot, 'agent-trust-report.json'), `${JSON.stringify({ generatedAt: new Date().toISOString(), harness_error: error instanceof Error ? error.message : String(error) }, null, 2)}\n`);
  throw error;
}
