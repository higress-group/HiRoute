import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { publishFeed } from '../scripts/model-feed.mjs';

const repository = path.resolve(import.meta.dirname, '../../..');
const snapshot = JSON.parse(await readFile(path.join(repository, 'assets/release-facts/current/bundle/official-model-data.json')));
const json = value => Buffer.from(`${JSON.stringify(value)}\n`);

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'hiroute-feed-oss-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sourceDirectory = path.join(root, 'source'), bin = path.join(root, 'bin');
  await mkdir(sourceDirectory);
  await mkdir(bin);
  const payload = { schema: 'hiroute.model-feed/v2', sequence: 2,
    issued_at: '2026-10-19T12:00:00Z', source_revision: 'a'.repeat(40), ...snapshot, featured: [] };
  for (const [name, value] of Object.entries({ 'payload.json': payload,
    'policy.json': { minimum_feed_client: '0.3.2' },
    'publication.json': { sequence: payload.sequence, issued_at: payload.issued_at }, 'featured.json': [] })) {
    await writeFile(path.join(sourceDirectory, name), json(value));
  }
  await writeFile(path.join(bin, 'aliyun'), `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$@" > "$FEED_UPLOAD_ARGS"
[[ "\${FEED_UPLOAD_FAIL:-}" != 1 ]] || exit 23
cp "$3" "$FEED_UPLOAD_OBJECT"
`, { mode: 0o755 });
  const distDirectory = path.join(root, 'feed-staging');
  return { sourceDirectory, distDirectory, destination: path.join(distDirectory, 'model-feed/v2.json'),
    payload, args: path.join(root, 'args'), object: path.join(root, 'served.json'), bin };
}

function upload(f, extra = {}) {
  return spawnSync('bash', [path.join(repository, '.github/scripts/deploy-model-feed-oss.sh'), f.destination], {
    cwd: repository, encoding: 'utf8', env: { ...process.env, PATH: `${f.bin}:${process.env.PATH}`,
      ACCESS_KEYID: 'fixture-id', ACCESS_KEYSECRET: 'fixture-secret',
      FEED_UPLOAD_ARGS: f.args, FEED_UPLOAD_OBJECT: f.object, ...extra },
  });
}

test('a standalone feed deploy uploads the exact complete snapshot to its one fixed OSS key', async t => {
  const f = await fixture(t);
  const result = await publishFeed({ ...f, env: { GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/heads/main' },
    fetchImpl: async () => new Response(null, { status: 404 }) });
  assert.equal(result.status, 'first_publication');
  const uploaded = upload(f);
  assert.equal(uploaded.status, 0, uploaded.stderr);
  assert.deepEqual(await readFile(f.object), json(f.payload));
  const args = (await readFile(f.args, 'utf8')).trim().split('\n');
  assert.deepEqual(args.slice(0, 4), ['oss', 'cp', f.destination, 'oss://hiroute-ai/model-feed/v2.json']);
  assert.ok(args.includes('Content-Type:application/json#Cache-Control:public,max-age=300'));
  assert.ok(!args.includes('--recursive'));
  assert.ok(f.payload.data.metadata_catalog.model_records.length > f.payload.data.models.length);
});

test('a missing staged payload fails before any OSS command', async t => {
  const f = await fixture(t);
  const result = upload(f);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Validated model feed missing/);
  await assert.rejects(readFile(f.args), { code: 'ENOENT' });
});

test('a failed single-object upload propagates failure without deleting the prior object', async t => {
  const f = await fixture(t);
  await mkdir(path.dirname(f.destination), { recursive: true });
  await writeFile(f.destination, json(f.payload));
  const previous = Buffer.from('previous served object');
  await writeFile(f.object, previous);
  assert.equal(upload(f, { FEED_UPLOAD_FAIL: '1' }).status, 23);
  assert.deepEqual(await readFile(f.object), previous);
});
