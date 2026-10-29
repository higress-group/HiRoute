import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const workflow = await readFile(new URL('../../../.github/workflows/model-feed.yml', import.meta.url), 'utf8');
const website = await readFile(new URL('../../../.github/workflows/website.yml', import.meta.url), 'utf8');
const release = await readFile(new URL('../../../.github/workflows/release.yml', import.meta.url), 'utf8');
const backend = await readFile(new URL('../../../.github/workflows/gateway-core.yml', import.meta.url), 'utf8');
const [, buildAndDeploy] = workflow.split('\njobs:\n');
const [validate, deploy] = buildAndDeploy.split('\n  deploy:\n');

function before(source, first, second) {
  assert.ok(source.includes(first), `Missing ${first}`);
  assert.ok(source.includes(second), `Missing ${second}`);
  assert.ok(source.indexOf(first) < source.indexOf(second), `${first} must precede ${second}`);
}

test('independent feed CI validates the complete metadata projections before freezing the payload', () => {
  assert.match(validate, /fetch-depth: 0/);
  for (const check of ['close_model_metadata.py --check', 'validate_catalog.py', 'generate.py --check', 'prepare-bundle.py --check']) {
    before(validate, check, 'cargo run --locked -p hiroute-release-facts -- feed build --check');
  }
  before(validate, 'cargo run --locked -p hiroute-release-facts -- feed build --check', 'model-feed.mjs check');
  before(validate, 'model-feed.mjs check', 'name: hiroute-model-feed-${{ github.sha }}');
  assert.match(validate, /MODEL_FEED_BASE: \$\{\{ github\.event\.pull_request\.base\.sha \|\| github\.event\.before \}\}/);
  assert.match(validate, /path: assets\/model-feed\/payload\.json/);
  assert.match(validate, /node --test apps\/website\/tests\/model-feed\*\.test\.mjs/);
  assert.doesNotMatch(validate, /secrets\.|model-feed\.mjs publish|environment:|npm |experiments\/|needs:.*website/);
  for (const pattern of ['assets/model-feed/**', 'assets/model-data/**', 'assets/release-facts/**',
    'contracts/model-feed/**', 'contracts/releases/**', 'crates/integrations/**', 'tools/model-metadata/**', 'tools/release-facts/**']) {
    assert.equal(workflow.split(`      - '${pattern}'`).length - 1, 2, `${pattern} must trigger PR and main builds`);
  }
});

test('production deploy consumes this revision artifact and uploads only a staged feed', () => {
  assert.match(deploy, /needs: validate/);
  assert.match(deploy, /if: github\.event_name != 'pull_request' && github\.ref == 'refs\/heads\/main'/);
  assert.match(deploy, /environment: website-production/);
  assert.match(deploy, /ref: \$\{\{ github\.sha \}\}/);
  assert.match(deploy, /name: hiroute-model-feed-\$\{\{ github\.sha \}\}/);
  before(deploy, 'path: ${{ runner.temp }}/validated-model-feed', 'model-feed.mjs publish --payload "$RUNNER_TEMP/validated-model-feed/payload.json"');
  before(deploy, 'model-feed.mjs publish', '.github/scripts/deploy-model-feed-oss.sh apps/website/dist/model-feed/v2.json');
  assert.match(deploy, /if \[\[ -f apps\/website\/dist\/model-feed\/v2\.json \]\]; then echo 'staged=true' >> "\$GITHUB_OUTPUT"; fi/);
  assert.equal(deploy.split("if: steps.feed.outputs.staged == 'true'").length - 1, 2);
  assert.doesNotMatch(deploy, /MODEL_FEED_SIGNING_KEYS|signing.key|Sign validated/i);
  assert.doesNotMatch(deploy, /deploy-website-oss|npm |needs:.*website/);
  assert.match(deploy, /concurrency:\n\s+group: hiroute-ai-production\n\s+cancel-in-progress: false/);
  assert.match(workflow, /cancel-in-progress: \$\{\{ github\.event_name == 'pull_request' \}\}/);
});

test('website build is independent and shares only the production deployment lock', () => {
  const [build, deploy] = website.split('\n  deploy:\n');
  assert.doesNotMatch(build, /hiroute-ai-production|ci-setup|cargo run|hiroute-model-feed/);
  assert.doesNotMatch(deploy, /model-feed\.mjs publish|hiroute-model-feed/);
  assert.match(deploy, /concurrency:\n\s+group: hiroute-ai-production\n\s+cancel-in-progress: false/);
  assert.doesNotMatch(website, /'assets\/model-(?:feed|data)\/\*\*'/);
});

test('feed, website and release uploads keep every pending production deployment', () => {
  for (const [name, source] of [['model-feed', workflow], ['website', website], ['release', release]]) {
    const production = [...source.matchAll(/^([ ]*)concurrency:\n((?:\1[ ]+[^\n]*(?:\n|$))+)/gm)]
      .map(match => match[2]).filter(block => /^ +group: hiroute-ai-production$/m.test(block));
    assert.equal(production.length, 1, `${name}: expected one production concurrency group`);
    assert.match(production[0], /^ +cancel-in-progress: false$/m, `${name}: running uploads must finish`);
    assert.match(production[0], /^ +queue: max$/m, `${name}: pending uploads must not replace each other`);
  }
});

test('backend metadata validation also uses the shared feed check with full release history', () => {
  const frontend = backend.split('\n  frontend:\n')[1];
  assert.match(frontend, /fetch-depth: 0/);
  before(frontend, 'prepare-bundle.py --check', 'cargo run -p hiroute-release-facts -- feed build --check');
  before(frontend, 'cargo run -p hiroute-release-facts -- feed build --check', 'git diff --exit-code');
});
