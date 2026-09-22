import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const repository = path.resolve(import.meta.dirname, '../../..');

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-oss-test-'));
  const assets = path.join(root, 'assets'), bin = path.join(root, 'bin'), dist = path.join(root, 'dist');
  fs.mkdirSync(assets); fs.mkdirSync(bin); fs.mkdirSync(dist);
  fs.writeFileSync(path.join(dist, 'index.html'), '<h1>HiRoute</h1>');
  const bytes = Buffer.from('not-a-real-dmg-publication-script-fixture');
  const revision = 'a'.repeat(40);
  const filename = `HiRoute-1.2.3-${revision.slice(0, 12)}-macos-arm64-trial.dmg`;
  fs.writeFileSync(path.join(assets, filename), bytes);
  const sha256 = crypto.createHash('sha256').update(bytes).digest('hex');
  const manifest = { schema: 'hiroute.website.releases/v2', releases: [{
    version: '1.2.3', channel: 'stable', published_at: '2026-09-21T00:00:00Z',
    notes: { zh: '版本。', en: 'Release.' }, artifacts: [{
      kind: 'desktop', platform: 'macOS', architecture: 'arm64', format: 'dmg', minimum_os: '15.0',
      distribution: 'self-signed', filename, sha256, size: bytes.length,
    }],
  }] };
  const manifestPath = path.join(root, 'releases.json');
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  const log = path.join(root, 'aliyun.log');
  const curlLog = path.join(root, 'curl.log');
  const state = path.join(root, 'uploaded');
  fs.writeFileSync(state, '');
  const fakeAliyun = `#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" >> \"$OSS_TEST_LOG\"\nif [[ \"$1 $2\" == 'oss cp' ]]; then touch \"$OSS_TEST_STATE\"; fi\n`;
  const fakeCurl = `#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" >> \"$CURL_TEST_LOG\"\noutput=\nwhile (( $# )); do\n  case \"$1\" in\n    --output) output=$2; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nif [[ ! -f \"$OSS_TEST_STATE\" ]]; then\n  printf 'HTTP/1.1 404 Not Found\\r\\n\\r\\n' > \"$output\"\n  printf '404'\n  exit 0\nfi\nprintf 'HTTP/1.1 200 OK\\r\\nContent-Length: %s\\r\\nx-oss-meta-sha256: %s\\r\\n\\r\\n' \"$OSS_TEST_SIZE\" \"$OSS_TEST_SHA\" > \"$output\"\nprintf '200'\n`;
  fs.writeFileSync(path.join(bin, 'aliyun'), fakeAliyun, { mode: 0o755 });
  fs.writeFileSync(path.join(bin, 'curl'), fakeCurl, { mode: 0o755 });
  return { root, assets, bin, dist, manifestPath, log, curlLog, state, filename, sha256, size: bytes.length };
}

function readLog(filename) {
  return fs.existsSync(filename) ? fs.readFileSync(filename, 'utf8') : '';
}

function execute(script, args, data, extraEnv = {}) {
  return spawnSync('bash', [path.join(repository, script), ...args], {
    cwd: repository, encoding: 'utf8', env: {
      ...process.env, PATH: `${path.join(data.root, 'bin')}:${process.env.PATH}`,
      ACCESS_KEYID: 'fixture-id', ACCESS_KEYSECRET: 'fixture-secret', OSS_TEST_LOG: data.log,
      CURL_TEST_LOG: data.curlLog,
      OSS_TEST_SHA: data.sha256, OSS_TEST_SIZE: String(data.size), OSS_TEST_STATE: data.state,
      ...extraEnv,
    },
  });
}

function run(script, args, data, extraEnv = {}) {
  const result = execute(script, args, data, extraEnv);
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

test('release publisher uses immutable hiroute-ai paths and SHA metadata', () => {
  const data = fixture();
  run('.github/scripts/publish-release-oss.sh', [data.assets, data.manifestPath, 'v1.2.3'], data);
  const log = readLog(data.log);
  assert.match(readLog(data.curlLog), new RegExp(`https://hiroute-ai\\.oss-cn-hongkong\\.aliyuncs\\.com/releases/1\\.2\\.3/${data.filename}`));
  assert.doesNotMatch(log, /oss cp/);
  assert.doesNotMatch(log, /oss stat/);
  assert.doesNotMatch(log, /higress-ai/);
});

test('release publisher creates only a missing object and never forces an overwrite', () => {
  const data = fixture();
  fs.rmSync(data.state);
  run('.github/scripts/publish-release-oss.sh', [data.assets, data.manifestPath, 'v1.2.3'], data);
  const log = readLog(data.log);
  assert.match(log, /oss cp/);
  assert.match(log, new RegExp(`X-Oss-Meta-Sha256:${data.sha256}`));
  assert.doesNotMatch(log, /--force/);
  assert.doesNotMatch(log, /oss stat/);
});

test('release publisher rejects an existing object with different immutable metadata', () => {
  const data = fixture();
  const result = execute('.github/scripts/publish-release-oss.sh',
    [data.assets, data.manifestPath, 'v1.2.3'], data, { OSS_TEST_SHA: '0'.repeat(64) });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /different SHA256/);
  assert.doesNotMatch(readLog(data.log), /oss cp/);
});

test('OSS object sizes must match the complete Content-Length field', () => {
  for (const [script, argumentsFor, message] of [
    ['.github/scripts/publish-release-oss.sh', data => [data.assets, data.manifestPath, 'v1.2.3'], /different size/],
    ['.github/scripts/deploy-website-oss.sh', data => [data.dist, data.manifestPath], /size does not match/],
  ]) {
    const data = fixture();
    const result = execute(script, argumentsFor(data), data, { OSS_TEST_SIZE: `${data.size}0` });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, message);
    assert.doesNotMatch(readLog(data.log), /oss cp/);
  }
});

test('website deploy checks release metadata and never targets a destructive sync', () => {
  const data = fixture();
  run('.github/scripts/deploy-website-oss.sh', [data.dist, data.manifestPath], data);
  const log = readLog(data.log);
  assert.match(readLog(data.curlLog), /https:\/\/hiroute-ai\.oss-cn-hongkong\.aliyuncs\.com\/releases\//);
  assert.match(log, /oss cp .* oss:\/\/hiroute-ai\//);
  assert.doesNotMatch(log, /oss stat/);
  assert.doesNotMatch(log, /\b(?:rm|sync)\b/);
  assert.doesNotMatch(log, /higress-ai/);
});

test('manifest generation failures stop OSS scripts before any remote command', () => {
  for (const [script, argumentsFor] of [
    ['.github/scripts/publish-release-oss.sh', data => [data.assets, data.manifestPath, 'v1.2.3']],
    ['.github/scripts/deploy-website-oss.sh', data => [data.dist, data.manifestPath]],
  ]) {
    const data = fixture();
    fs.writeFileSync(data.manifestPath, '{"schema":"invalid","releases":[]}');
    const result = execute(script, argumentsFor(data), data);
    assert.notEqual(result.status, 0);
    assert.equal(fs.existsSync(data.log), false);
    assert.equal(fs.existsSync(data.curlLog), false);
  }
});
