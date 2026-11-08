import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connectPage, evaluate, navigate, waitFor } from './cdp-client.mjs';
import { decisionModelCaptures } from './decision-model-captures.mjs';

const port = Number(process.argv[2]);
const baseUrl = process.argv[3];
const output = path.resolve(process.argv[4]);
await mkdir(output, { recursive: true });
const client = await connectPage(port);
try {
  const checks = [];
  if (process.env.HIROUTE_UI_CAPTURE_BASELINE !== '1') {
    await navigate(client, new URL('product-shell.html', baseUrl).href, { width: 1200, height: 813 });
    await waitFor(client, 'Boolean(window.productShell)');
    const ids = ['desktop.decisions.details-and-removal', 'desktop.decisions.extension-removal', 'desktop.decisions.removal-references', 'desktop.decisions.removal-preview-states'];
    for (const id of ids) {
      const result = await evaluate(client, `import('/product-shell-scenarios.mjs').then(module => module.runProductShellScenarios(${JSON.stringify({ ids: [id], requiredIds: [id] })}))`);
      checks.push(...result.results);
      if (result.failed) throw new Error(JSON.stringify(result));
      console.log(`green: ${id}`);
    }
  }
  process.env.HIROUTE_DECISION_CAPTURE_STATES ??= 'removal,removal-blocked,removal-loading,removal-error';
  checks.push(...await decisionModelCaptures(client, baseUrl, output));
  await writeFile(path.join(output, 'result.json'), JSON.stringify({ checks }, null, 2));
} finally {
  client.close();
}
