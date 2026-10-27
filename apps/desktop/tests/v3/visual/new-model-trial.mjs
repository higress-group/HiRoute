import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktopRoot = path.resolve(here, '../../..');
const outputRoot = path.resolve(process.argv[2] ?? path.join(desktopRoot, 'test-results/v3-new-model-trial'));

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
  // Browser helpers may outlive the browser while retaining its inherited pipes.
  child.stdout?.destroy();
  child.stderr?.destroy();
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
const profile = await mkdtemp(path.join(os.tmpdir(), 'hiroute-v3-new-model-trial-'));
await mkdir(outputRoot, { recursive: true });
let vite;
let chrome;
let viteLog = '';
const captureVite = chunk => { viteLog = (viteLog + chunk.toString()).slice(-65536); };
try {
  vite = spawn(path.join(desktopRoot, 'node_modules/.bin/vite'), ['tests/v3/browser', '--host', '127.0.0.1', '--port', String(httpPort), '--strictPort'], { cwd: desktopRoot, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  vite.stdout.on('data', captureVite);
  vite.stderr.on('data', captureVite);
  await new Promise((resolve, reject) => { vite.once('spawn', resolve); vite.once('error', reject); });
  await waitFor(`http://127.0.0.1:${httpPort}/`);
  const chromeExecutable = process.env.CHROME_BIN || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
  chrome = spawn(chromeExecutable, ['--headless=new', '--disable-gpu', `--remote-debugging-port=${cdpPort}`, '--remote-allow-origins=*', `--user-data-dir=${profile}`, '--window-size=1280,900', 'about:blank'], { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { chrome.once('spawn', resolve); chrome.once('error', reject); });
  await waitFor(`http://127.0.0.1:${cdpPort}/json/version`);
  const selectedRunner = [
    ['--session-live-refresh', 'session-live-refresh-active.mjs'],
    ['--presentation', 'timeline-presentation-active.mjs'],
    ['--metadata', 'model-metadata-active.mjs'],
    ['--protection', 'new-model-trial-protection-active.mjs'],
    ['--polish', 'new-model-trial-polish-active.mjs'],
  ].find(([flag]) => process.argv.includes(flag))?.[1] ?? 'new-model-trial-active.mjs';
  const runner = spawn(process.execPath, [path.join(here, selectedRunner), String(cdpPort), `http://127.0.0.1:${httpPort}/`, outputRoot, ...['--timeline-pricing','--trial-agent-reads'].filter(flag=>process.argv.includes(flag))], { cwd: desktopRoot, stdio: 'inherit' });
  const outcome = await waitForExit(runner);
  if (outcome.code !== 0) process.exitCode = outcome.code ?? 1;
} finally {
  await terminate(chrome);
  await terminate(vite);
  await writeFile(path.join(outputRoot, 'vite.log'), viteLog);
  await removeProfile(profile);
}
