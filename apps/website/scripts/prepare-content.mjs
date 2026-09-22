import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { generateLinuxInstallScript } from '../src/lib/install-script.mjs';
import { releaseManifest } from '../src/lib/releases.mjs';

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repository = path.resolve(website, '../..');
const source = path.join(repository, 'decision-extensions');
const publicDirectory = path.join(website, 'public');
const apiDirectory = path.join(publicDirectory, 'api');
const assetsDirectory = path.join(publicDirectory, 'decision-assets');
const installDirectory = path.join(publicDirectory, 'install');

await fs.rm(apiDirectory, { recursive: true, force: true });
await fs.rm(assetsDirectory, { recursive: true, force: true });
await fs.rm(installDirectory, { recursive: true, force: true });
await fs.mkdir(apiDirectory, { recursive: true });
await fs.mkdir(assetsDirectory, { recursive: true });
await fs.mkdir(installDirectory, { recursive: true });
// Reuse the Desktop logo for raster-only link preview services.
await fs.copyFile(path.join(repository, 'apps/desktop/src-tauri/icons/icon.png'), path.join(publicDirectory, 'brand/app-icon.png'));
await fs.copyFile(path.join(source, 'api/decision.openapi.json'), path.join(apiDirectory, 'decision.openapi.json'));
for (const entry of await fs.readdir(path.join(source, 'assets'), { withFileTypes: true })) {
  if (entry.isFile() && /\.(png|svg)$/.test(entry.name)) {
    await fs.copyFile(path.join(source, 'assets', entry.name), path.join(assetsDirectory, entry.name));
  }
}
await fs.copyFile(path.join(repository, 'scripts/install-standalone.py'), path.join(installDirectory, 'standalone.py'));
await fs.writeFile(path.join(publicDirectory, 'install.sh'), generateLinuxInstallScript(releaseManifest), { mode: 0o755 });
console.log('Prepared canonical Decision API, illustrations, and Linux installer entry.');
