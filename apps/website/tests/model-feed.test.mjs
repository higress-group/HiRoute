import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { checkFeed, checkPayload, FEED_URL, MAX_FEED_BYTES, payloadDigest, publishFeed } from '../scripts/model-feed.mjs';

const repository = path.resolve(import.meta.dirname, '../../..');
const script = path.join(repository, 'apps/website/scripts/model-feed.mjs');
const production = { GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/heads/main' };
const snapshot = JSON.parse(readFileSync(path.join(repository, 'assets/release-facts/current/bundle/official-model-data.json')));
const payload = (sequence = 2, extra = {}) => ({ schema: 'hiroute.model-feed/v2', sequence,
  issued_at: '2026-10-18T08:00:00Z', source_revision: 'a'.repeat(40), ...structuredClone(snapshot), featured: [], ...extra });
const json = value => Buffer.from(`${JSON.stringify(value)}\n`);
const response = bytes => new Response(bytes, { status: 200 });
const firstPublication = async () => new Response(null, { status: 404 });
const noRequest = async () => assert.fail('This path must not make a network request');

async function fixture(t, value = payload()) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'hiroute-feed-test-'));
  t.after(() => rm(root, { force: true, recursive: true }));
  const sourceDirectory = path.join(root, 'source');
  const distDirectory = path.join(root, 'dist');
  await mkdir(sourceDirectory);
  await mkdir(distDirectory);
  await writeFile(path.join(distDirectory, 'index.html'), '<h1>Website</h1>');
  for (const [name, content] of Object.entries({ 'payload.json': value,
    'policy.json': { minimum_feed_client: '0.3.2' },
    'publication.json': { sequence: value?.sequence ?? 1, issued_at: value?.issued_at ?? '2026-10-18T08:00:00Z' },
    'featured.json': value?.featured ?? [] })) await writeFile(path.join(sourceDirectory, name), json(content));
  return { sourceDirectory, distDirectory, destination: path.join(distDirectory, 'model-feed/v2.json'),
    summary: path.join(root, 'summary.md'), env: production };
}

async function noOutput(f) {
  await assert.rejects(readFile(f.destination), { code: 'ENOENT' });
  assert.equal(await readFile(path.join(f.distDirectory, 'index.html'), 'utf8'), '<h1>Website</h1>');
}

test('bootstrap works without network and leaves the ordinary website deployable', async t => {
  const f = await fixture(t, null);
  const checked = await checkFeed(f);
  assert.equal(checked.payload, null);
  const result = await publishFeed({ ...f, env: production, fetchImpl: noRequest });
  assert.deepEqual(result, { status: 'bootstrap', sequence: null, data_revision: null, data_digest: null, payload_digest: null, source_revision: null });
  await noOutput(f);
});

test('PR checking requires no secrets; publishing rejects PR and non-main contexts before fetch', async t => {
  const f = await fixture(t);
  assert.equal((await checkFeed(f)).payload.sequence, 2n);
  for (const env of [{ GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/1/merge' },
    { GITHUB_EVENT_NAME: 'pull_request_target', GITHUB_REF: 'refs/heads/main' },
    { GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/feature' }, {}]) {
    await assert.rejects(publishFeed({ ...f, env, fetchImpl: noRequest }), /only/);
    await noOutput(f);
  }
  const cli = spawnSync(process.execPath, [script, 'publish'], { encoding: 'utf8',
    env: { ...process.env, GITHUB_EVENT_NAME: 'pull_request' } });
  assert.equal(cli.status, 1);
  assert.match(cli.stderr, /only a production/);
});

test('first 404 publishes exact validated JSON bytes without keys and reports provenance', async t => {
  const f = await fixture(t);
  let calls = 0;
  const result = await publishFeed({ ...f, env: { ...production, GITHUB_STEP_SUMMARY: f.summary },
    fetchImpl: async (url, options) => {
      calls += 1;
      assert.equal(url, FEED_URL);
      assert.equal(options.redirect, 'error');
      assert.equal(options.credentials, 'omit');
      assert.equal(options.cache, 'no-store');
      assert.equal(options.headers['Cache-Control'], 'no-cache');
      assert.deepEqual(Object.keys(options.headers).sort(), ['Accept', 'Cache-Control']);
      return firstPublication();
    } });
  assert.equal(calls, 1);
  assert.equal(result.status, 'first_publication');
  const bytes = await readFile(f.destination);
  assert.equal(checkPayload(bytes).sequence, 2n);
  assert.equal(result.payload_digest, payloadDigest(json(payload())));
  assert.deepEqual(bytes, json(payload()));
  assert.equal(JSON.parse(bytes).schema, 'hiroute.model-feed/v2');
  assert.equal(Object.hasOwn(JSON.parse(bytes), 'signatures'), false);
  const summary = await readFile(f.summary, 'utf8');
  assert.match(summary, /Sequence: 2/);
  assert.ok(summary.includes(result.payload_digest));
  assert.ok(summary.includes('a'.repeat(40)));
  assert.doesNotMatch(summary, /PRIVATE KEY/);
});

test('a lower served sequence accepts a newer publication', async t => {
  const f = await fixture(t);
  const result = await publishFeed({ ...f, fetchImpl: async () => response(json(payload(1))) });
  assert.equal(result.status, 'published');
  assert.equal(checkPayload(await readFile(f.destination)).sequence, 2n);
});

test('matching payload digest is a no-op and does not rewrite the served feed', async t => {
  const f = await fixture(t);
  const served = json(payload());
  const before = Buffer.from(served);
  const result = await publishFeed({ ...f, env: production,
    fetchImpl: async () => response(served) });
  assert.equal(result.status, 'already_current');
  assert.deepEqual(served, before);
  await noOutput(f);
});

test('older proposed sequence and equal sequence with different content never write an output', async t => {
  for (const served of [payload(3), payload(2, { source_revision: 'b'.repeat(40) })]) {
    const f = await fixture(t);
    await assert.rejects(publishFeed({ ...f, fetchImpl: async () => response(json(served)) }), /sequence must exceed/);
    await noOutput(f);
  }
});

test('invalid served JSON, schemas and forbidden record kinds preserve the served publication', async t => {
  for (const bytes of [Buffer.from('invalid JSON'), json(null), json({}),
    json(payload(1, { schema: 'hiroute.model-feed/v99' })),
    json(payload(1, { records: [{ kind: 'connector', record: {} }] })),
    json({ schema: 'hiroute.model-feed.envelope/v1', payload: 'e30=', signatures: [] })]) {
    const f = await fixture(t);
    await assert.rejects(publishFeed({ ...f, fetchImpl: async () => response(bytes) }));
    await noOutput(f);
  }
});

test('only initial 404 permits first publication; other HTTP, redirect, and network failures stop it', async t => {
  for (const fetchImpl of [async () => new Response(null, { status: 403 }),
    async () => new Response(null, { status: 500 }), async () => new Response(null, { status: 304 }),
    async () => new Response(null, { status: 302, headers: { Location: 'https://example.com/feed' } }),
    async () => { throw new Error('network offline'); }]) {
    const f = await fixture(t);
    await assert.rejects(publishFeed({ ...f, fetchImpl }), /served feed:/);
    await noOutput(f);
  }
});

test('payload and streaming response enforce the 16 MiB bound even without content length', async t => {
  assert.throws(() => checkPayload(Buffer.alloc(MAX_FEED_BYTES + 1)), /exceeds 16 MiB/);
  for (const withHeader of [false, true]) {
    const f = await fixture(t);
    const result = new Response(new ReadableStream({ start(controller) {
      controller.enqueue(new Uint8Array(MAX_FEED_BYTES));
      controller.enqueue(new Uint8Array(1));
      controller.close();
    } }), { headers: withHeader ? { 'Content-Length': String(MAX_FEED_BYTES + 1) } : {} });
    await assert.rejects(publishFeed({ ...f, fetchImpl: async () => result }), /exceeds 16 MiB/);
    await noOutput(f);
  }
});

test('a build artifact containing a feed is refused even for a bootstrap/no-op deployment', async t => {
  const f = await fixture(t, null);
  await mkdir(path.dirname(f.destination));
  await writeFile(f.destination, 'unexpected artifact');
  await assert.rejects(publishFeed({ ...f, fetchImpl: noRequest }), /dist already contains a feed/);
  assert.equal(await readFile(f.destination, 'utf8'), 'unexpected artifact');
});

test('sequence values retain the full u64 range without rounding or accepting non-integers', () => {
  for (const sequence of ['9007199254740993', '18446744073709551615']) {
    const bytes = Buffer.from(json(payload()).toString().replace('"sequence":2', `"sequence":${sequence}`));
    assert.equal(checkPayload(bytes).sequence, BigInt(sequence));
  }
  for (const sequence of ['0', '-1', '1.5', '1e3', '18446744073709551616', '"2"']) {
    assert.throws(() => checkPayload(Buffer.from(json(payload()).toString().replace('"sequence":2', `"sequence":${sequence}`))), /sequence/);
  }
});

test('payload structure rejects invalid dates, kinds, duplicate IDs, and unsafe featured links', () => {
  const featured = { id: 'test-entry', kind: 'new_model', model_configuration_id: 'model.test',
    headline: { en: 'Test model', zh: '测试模型' }, note: { en: 'Test announcement', zh: '测试公告' },
    links: ['https://hiroute.ai/en/docs/'], included_on: '2026-10-18' };
  const record = { kind: 'price_rate', min_client_version: '0.3.2', included_on: '2026-10-18',
    record: { price_rate_id: 'price.test' } };
  assert.equal(checkPayload(json(payload(2, { featured: [featured] }))).sequence, 2n);
  for (const extra of [{ schema: 'unknown' }, { source_revision: 'main' }, { issued_at: '2026-02-30T00:00:00Z' },
    { records: [{ ...record, kind: 'connector' }] }, { data_revision: 'latest' },
    { records: [record, record] }, { featured: [featured, featured] }, { unknown: true },
    ...['https://example.com/', 'https://hiroute.ai.evil/', 'https://hiroute.ai@evil/', 'http://hiroute.ai/',
      'https://hiroute.ai/\\evil', 'https://hiroute.ai/\n@evil/'].map(link => ({ featured: [{ ...featured, links: [link] }] }))]) {
    assert.throws(() => checkPayload(json(payload(2, extra))));
  }
});

test('source metadata mismatch and oversized payloads fail in the build check', async t => {
  const f = await fixture(t);
  await writeFile(path.join(f.sourceDirectory, 'publication.json'), json({ sequence: 3, issued_at: payload().issued_at }));
  await assert.rejects(checkFeed(f), /publication metadata differs/);
  await writeFile(path.join(f.sourceDirectory, 'publication.json'), json({ sequence: 2, issued_at: payload().issued_at }));
  const bytes = Buffer.concat([json(payload()), Buffer.alloc(MAX_FEED_BYTES, ' ')]);
  await writeFile(path.join(f.sourceDirectory, 'payload.json'), bytes);
  await assert.rejects(checkFeed(f), /exceeds 16 MiB/);
});

test('duplicate JSON fields are rejected, including escaped spellings', () => {
  const duplicate = Buffer.from(json(payload()).toString().replace('"sequence":2', '"sequence":1,"seque\\u006ece":2'));
  assert.throws(() => checkPayload(duplicate), /invalid JSON/);
});

test('exact payload size is accepted; invalid UTF-8 and BOM bytes are refused', () => {
  const bytes = json(payload());
  assert.equal(checkPayload(Buffer.concat([bytes, Buffer.alloc(MAX_FEED_BYTES - bytes.length, ' ')])).sequence, 2n);
  assert.throws(() => checkPayload(Buffer.from([0xff])), /invalid UTF-8/);
  assert.throws(() => checkPayload(Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), json(payload())])), /invalid JSON/);
});
