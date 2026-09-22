import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktopRoot = path.resolve(here, '../../..');
const outputRoot = path.resolve(process.argv[2] ?? path.join(desktopRoot, 'test-results/v3-agent-trust'));
const routeSaveOnly = process.argv[3] === '--route-save-only';
const workerReplacementOnly = process.argv[3] === '--worker-replacement-only';
const claudeCollaborationOnly = process.argv[3] === '--claude-collaboration-only';

const availablePort = () => new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once('error', reject);
  server.listen(0, '127.0.0.1', () => {
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;
    server.close(error => error ? reject(error) : resolve(port));
  });
});
const waitFor = async (url, timeout = 15_000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 80));
  }
  throw new Error(`Timed out waiting for ${url}`);
};
const terminate = async child => {
  if (!child?.pid) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await Promise.race([
    new Promise(resolve => child.once('exit', resolve)),
    new Promise(resolve => setTimeout(resolve, 1500)),
  ]);
};
const removeProfile = async directory => {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try { await rm(directory, { recursive: true, force: true }); return; } catch (error) {
      if (attempt === 4) throw error;
      await new Promise(resolve => setTimeout(resolve, 120));
    }
  }
};
const waitForExit = child => new Promise((resolve, reject) => {
  child.once('error', reject);
  child.once('exit', (code, signal) => resolve({ code, signal }));
});

const httpPort = await availablePort();
let cdpPort = await availablePort();
while (cdpPort === httpPort) cdpPort = await availablePort();
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-v3-agent-trust-'));
await mkdir(outputRoot, { recursive: true });
let vite;
let chrome;
try {
  vite = spawn(path.join(desktopRoot, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktopRoot, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await waitFor(`http://127.0.0.1:${httpPort}/`);
  const chromeExecutable = process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
  chrome = spawn(chromeExecutable, ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, '--window-size=1280,900', 'about:blank'], { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await waitFor(`http://127.0.0.1:${cdpPort}/json/version`);
  const runner = spawn(process.execPath, [path.join(here, 'agent-trust-active.mjs'), String(cdpPort), `http://127.0.0.1:${httpPort}/`, outputRoot, ...(routeSaveOnly ? ['--route-save-only'] : workerReplacementOnly ? ['--worker-replacement-only'] : claudeCollaborationOnly ? ['--claude-collaboration-only'] : [])], { cwd: desktopRoot, stdio: 'inherit' });
  const outcome = await waitForExit(runner);
  if (outcome.code !== 0) process.exitCode = outcome.code ?? 1;
} finally {
  await terminate(chrome);
  await terminate(vite);
  await removeProfile(profile);
}
