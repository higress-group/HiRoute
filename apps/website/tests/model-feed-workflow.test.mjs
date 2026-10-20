import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const workflow = await readFile(new URL('../../../.github/workflows/website.yml', import.meta.url), 'utf8');
const backend = await readFile(new URL('../../../.github/workflows/gateway-core.yml', import.meta.url), 'utf8');
const [, buildAndDeploy] = workflow.split('\njobs:\n');
const [build, deploy] = buildAndDeploy.split('\n  deploy:\n');

function before(source, first, second) {
  assert.ok(source.includes(first), `Missing ${first}`);
  assert.ok(source.includes(second), `Missing ${second}`);
  assert.ok(source.indexOf(first) < source.indexOf(second), `${first} must precede ${second}`);
}

test('website PR build validates with shared Rust admission before uploading the validated payload', () => {
  assert.match(build, /fetch-depth: 0/);
  before(build, 'cargo run -p hiroute-release-facts -- feed build --check', 'model-feed.mjs check');
  before(build, 'model-feed.mjs check', 'name: hiroute-model-feed-${{ github.sha }}');
  assert.match(build, /path: assets\/model-feed\/payload\.json/);
  assert.match(build, /path: apps\/website\/dist/);
  assert.doesNotMatch(build, /secrets\.|MODEL_FEED_SIGNING_KEYS|model-feed\.mjs publish|environment:/);
  for (const pattern of ['assets/model-feed/**', 'assets/model-data/**', 'assets/release-facts/**',
    'contracts/releases/**', 'crates/integrations/**', 'tools/release-facts/**']) {
    assert.equal(workflow.split(`      - '${pattern}'`).length - 1, 2, `${pattern} must trigger PR and main builds`);
  }
});

test('production deploy consumes this revision artifact before the existing OSS upload', () => {
  assert.match(deploy, /needs: build/);
  assert.match(deploy, /if: github\.event_name != 'pull_request' && github\.ref == 'refs\/heads\/main'/);
  assert.match(deploy, /environment: website-production/);
  assert.match(deploy, /name: hiroute-model-feed-\$\{\{ github\.sha \}\}/);
  before(deploy, 'path: ${{ runner.temp }}/validated-model-feed', 'model-feed.mjs publish --payload "$RUNNER_TEMP/validated-model-feed/payload.json"');
  before(deploy, 'model-feed.mjs publish', '.github/scripts/deploy-website-oss.sh apps/website/dist apps/website/data/releases.json');
  assert.doesNotMatch(deploy, /MODEL_FEED_SIGNING_KEYS|signing.key|Sign validated/i);
  assert.match(workflow, /\|\| 'hiroute-ai-production'/);
  assert.match(workflow, /cancel-in-progress: \$\{\{ github\.event_name == 'pull_request' \}\}/);
});

test('backend metadata validation also uses the shared feed check with full release history', () => {
  const frontend = backend.split('\n  frontend:\n')[1];
  assert.match(frontend, /fetch-depth: 0/);
  before(frontend, 'prepare-bundle.py --check', 'cargo run -p hiroute-release-facts -- feed build --check');
  before(frontend, 'cargo run -p hiroute-release-facts -- feed build --check', 'git diff --exit-code');
});
