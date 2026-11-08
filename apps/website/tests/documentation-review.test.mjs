import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { documentationSnapshot, prepareDocumentationReview, checkDocumentationReview,
  checkAllDocumentationReviews } from '../scripts/documentation-review.mjs';

const tag = 'v9.9.9', revision = 'a'.repeat(40);
const script = fileURLToPath(new URL('../scripts/documentation-review.mjs', import.meta.url));
const reviewPath = `apps/website/docs/release-reviews/${tag}.json`;
function write(root, file, value) {
  fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
  fs.writeFileSync(path.join(root, file), typeof value === 'string' ? value : JSON.stringify(value));
}
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-doc-review-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const file of ['README.md', 'README.zh-CN.md', 'apps/website/content/guides/example.en.md',
    'apps/website/content/guides/example.zh.md', 'apps/website/src/components/GuidePage.astro',
    'apps/website/public/scripts/guide-agent-copy.js', 'decision-extensions/README.md',
    'decision-extensions/api/decision.openapi.json', 'apps/website/public/guide-assets/example-en.webp',
    'apps/website/public/guide-assets/example-zh.webp', 'decision-extensions/assets/readme-trial-en.png',
    'apps/website/docs/guide-assets.md']) write(root, file, `fixture only: ${file}`);
  return root;
}
function reviewed(root) {
  const draft = prepareDocumentationReview(root, tag, revision);
  draft.guides.status = 'updated';
  draft.guides.summary = 'Updated the bilingual example steps and their CLI pointers.';
  draft.guides.reviewed_files = ['apps/website/content/guides/example.en.md', 'apps/website/content/guides/example.zh.md'];
  draft.screenshots.status = 'unchanged';
  draft.screenshots.summary = 'Checked both captures; the displayed controls and layout are unchanged.';
  draft.screenshots.reviewed_files = ['apps/website/public/guide-assets/example-en.webp', 'apps/website/public/guide-assets/example-zh.webp'];
  write(root, reviewPath, draft);
  return draft;
}

test('a new release requires a completed, exact-revision review of both guides and screenshots', t => {
  const root = fixture(t);
  assert.throws(() => checkDocumentationReview(root, tag, revision), /missing documentation review/);
  write(root, reviewPath, prepareDocumentationReview(root, tag, revision));
  assert.throws(() => checkDocumentationReview(root, tag, revision), /guides review is pending/);
  const review = reviewed(root);
  assert.equal(checkDocumentationReview(root, tag, revision).status, 'reviewed');
  assert.equal(checkAllDocumentationReviews(root).length, 1);
  for (const change of [
    value => { value.product_revision = 'b'.repeat(40); },
    value => { value.tag = 'v9.9.8'; },
    value => { value.screenshots.status = 'pending'; },
    value => { value.screenshots.summary = ''; },
    value => { value.guides.reviewed_files = []; },
    value => { value.guides.reviewed_files = ['../private.txt']; },
  ]) {
    const invalid = structuredClone(review); change(invalid); write(root, reviewPath, invalid);
    assert.throws(() => checkDocumentationReview(root, tag, revision));
  }
});

test('any documentation, screenshot or copy-interaction change invalidates the reviewed snapshot', t => {
  const root = fixture(t);
  for (const file of ['apps/website/content/guides/example.en.md', 'apps/website/public/scripts/guide-agent-copy.js',
    'apps/website/public/guide-assets/example-zh.webp', 'apps/website/docs/guide-assets.md']) {
    reviewed(root);
    write(root, file, 'changed after review');
    assert.throws(() => checkDocumentationReview(root, tag, revision), /changed after review/);
  }
  reviewed(root);
  write(root, 'apps/website/content/guides/another.en.md', 'new guide');
  assert.throws(() => checkDocumentationReview(root, tag, revision), /guides changed after review/);
  reviewed(root);
  fs.unlinkSync(path.join(root, 'apps/website/public/guide-assets/example-zh.webp'));
  assert.throws(() => checkDocumentationReview(root, tag, revision), /reviewed screenshots files|changed after review/);
});

test('review generation is deterministic and never fabricates a completed assessment', t => {
  const root = fixture(t);
  assert.deepEqual(documentationSnapshot(root), documentationSnapshot(root));
  const first = prepareDocumentationReview(root, tag, revision);
  const second = prepareDocumentationReview(root, tag, revision);
  assert.deepEqual(first, second);
  assert.equal(first.guides.status, 'pending');
  assert.equal(first.screenshots.status, 'pending');
  assert.ok(!fs.existsSync(path.join(root, reviewPath)), 'prepare must only return a draft');
});

test('recorded release retries retain history, but another revision needs a new review', t => {
  const root = fixture(t);
  write(root, `contracts/releases/${tag}.json`, { schema: 'hiroute.release-contract-snapshot/v1', release: { tag, revision } });
  assert.equal(checkDocumentationReview(root, tag, revision).status, 'recorded-release');
  const replacement = 'b'.repeat(40);
  assert.throws(() => checkDocumentationReview(root, tag, replacement), /missing documentation review/);
  const review = reviewed(root); review.product_revision = replacement; write(root, reviewPath, review);
  assert.equal(checkDocumentationReview(root, tag, replacement).status, 'reviewed');
});

test('the publication CLI fails closed before a review, then accepts the reviewed source', t => {
  const root = fixture(t);
  const args = [script, 'check', '--repo', root, '--tag', tag, '--revision', revision];
  const before = spawnSync(process.execPath, args, { encoding: 'utf8' });
  assert.notEqual(before.status, 0); assert.match(before.stderr, /missing documentation review/);
  reviewed(root);
  const after = spawnSync(process.execPath, args, { encoding: 'utf8' });
  assert.equal(after.status, 0, after.stderr); assert.equal(JSON.parse(after.stdout).status, 'reviewed');
  const invalid = spawnSync(process.execPath, [script, 'check', '--tag', '../private', '--revision', revision], { encoding: 'utf8' });
  assert.notEqual(invalid.status, 0);
});

test('release publication checks documentation before downloading or uploading assets', () => {
  const workflow = fs.readFileSync(new URL('../../../.github/workflows/release.yml', import.meta.url), 'utf8');
  const gate = workflow.indexOf('node apps/website/scripts/documentation-review.mjs check --tag "$RELEASE_TAG" --revision "$revision"');
  assert.ok(gate > 0 && gate < workflow.indexOf('gh release download'));
  assert.match(workflow, /publish:\n\s+needs: verify-release/);
});
