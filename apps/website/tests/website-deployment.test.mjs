import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

const repository = path.resolve(import.meta.dirname, '../../..');
const command = path.join(repository, '.github/scripts/website-deployment.mjs');
const catalog = 'apps/website/data/releases.json';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-website-deploy-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const bin = path.join(root, 'bin');
  fs.mkdirSync(bin);
  const output = path.join(root, 'output');
  const githubCalls = path.join(root, 'github-calls');

  function write(filename, text) {
    const file = path.join(root, filename);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text);
  }

  function git(...args) {
    return execFileSync('git', [
      '-c', 'user.name=Website Test', '-c', 'user.email=test@example.com',
      '-c', 'core.hooksPath=/dev/null', ...args,
    ], { cwd: root, encoding: 'utf8' }).trim();
  }

  function commit(message) {
    git('add', 'apps', 'contracts');
    git('commit', '--quiet', '-m', message);
    return git('rev-parse', 'HEAD');
  }

  function writeCatalog(versions, notes = 'Release.') {
    write(catalog, JSON.stringify({
      schema: 'hiroute.website.releases/v3',
      releases: versions.map(version => ({
        version, channel: 'stable', published_at: '2026-09-21',
        notes: { zh: '发行。', en: notes },
        artifacts: [{
          kind: 'desktop', platform: 'macOS', architecture: 'arm64', format: 'dmg',
          minimum_os: '15.0', distribution: 'self-signed',
          filename: `HiRoute-${version}-aaaaaaaaaaaa-macos-arm64-trial.dmg`,
          sha256: 'b'.repeat(64), size: 123,
        }],
      })),
    }));
  }

  function run(args, { published = 'v7.0.0\n', apiStatus = '0' } = {}) {
    fs.rmSync(output, { force: true });
    fs.rmSync(githubCalls, { force: true });
    const result = spawnSync(process.execPath, [command, ...args], {
      cwd: root, encoding: 'utf8', env: {
        ...process.env, PATH: `${bin}:${process.env.PATH}`,
        GITHUB_OUTPUT: output, GITHUB_REPOSITORY: 'example/website',
        GH_TEST_CALLS: githubCalls, GH_TEST_PUBLISHED: published, GH_TEST_STATUS: apiStatus,
      },
    });
    return {
      status: result.status, stdout: result.stdout, stderr: result.stderr,
      output: fs.existsSync(output) ? fs.readFileSync(output, 'utf8') : '',
      github: fs.existsSync(githubCalls) ? fs.readFileSync(githubCalls, 'utf8') : '',
    };
  }

  git('init', '--quiet', '-b', 'main');
  write('bin/gh', `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$@" > "$GH_TEST_CALLS"
if [[ "$GH_TEST_STATUS" != 0 ]]; then
  printf '%s\\n' 'GitHub API unavailable' >&2
  exit "$GH_TEST_STATUS"
fi
printf '%s' "$GH_TEST_PUBLISHED"
`);
  fs.chmodSync(path.join(bin, 'gh'), 0o755);
  write('contracts/releases/index.v1.json', JSON.stringify({ releases: [{ tag: 'v7.0.0' }] }));
  writeCatalog(['7.0.0']);
  const base = commit('recorded release');
  return { root, base, git, commit, write, writeCatalog, run };
}

function assertDeploy(result, expected) {
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.output, `deploy=${expected}\n`);
}

test('a push adding an unrecorded version defers deployment even when it also updates the site', t => {
  const repo = fixture(t);
  repo.writeCatalog(['7.0.1', '7.0.0']);
  repo.commit('add package catalog');
  repo.write('apps/website/content/guide.md', 'Updated guide');
  repo.commit('update website content in the same push');

  // The Release might be public by the time the build gets to this step.
  const result = repo.run(['plan', repo.base], { published: 'v7.0.1\nv7.0.0\n' });
  assertDeploy(result, false);
  assert.match(result.stdout, /publish-release.*v7\.0\.1/);
  assert.equal(result.github, '');
});

test('follow-up website builds wait for a pending tag, then use the ordinary deployment path', t => {
  const repo = fixture(t);
  repo.writeCatalog(['7.0.1', '7.0.0']);
  const catalogCommit = repo.commit('add package catalog');
  repo.write('apps/website/content/guide.md', 'Updated guide');
  repo.commit('update website while release is pending');

  for (const args of [['plan', catalogCommit], ['plan', '']]) {
    const pending = repo.run(args);
    assertDeploy(pending, false);
    assert.match(pending.stdout, /GitHub publication of v7\.0\.1/);
    assert.match(pending.github, /api\n--paginate\nrepos\/example\/website\/releases\?per_page=100/);
    assert.match(pending.github, /select\(\.draft == false and \.published_at != null\)/);
    assertDeploy(repo.run(args, { published: 'v7.0.1\nv7.0.0\n' }), true);
  }
  const unavailable = repo.run(['plan', catalogCommit], { apiStatus: '1' });
  assert.notEqual(unavailable.status, 0);
  assert.equal(unavailable.output, '');
  assert.match(unavailable.stderr, /GitHub API unavailable/);
});

test('edits to a recorded release or the website still deploy without querying GitHub', t => {
  const repo = fixture(t);
  repo.writeCatalog(['7.0.0'], 'Updated release notes.');
  repo.write('apps/website/content/guide.md', 'Updated guide');
  repo.commit('edit published release notes and guide');
  const result = repo.run(['plan', repo.base], { apiStatus: '1' });
  assertDeploy(result, true);
  assert.equal(result.github, '');
});

test('production rechecks queued website builds against main and allows unrelated changes', t => {
  const repo = fixture(t);
  const origin = path.join(repo.root, 'origin.git');
  repo.git('init', '--quiet', '--bare', origin);
  repo.git(`--git-dir=${origin}`, 'config', 'core.hooksPath', '/dev/null');
  repo.git('remote', 'add', 'origin', origin);
  repo.write('contracts/releases/index.v1.json', JSON.stringify({
    releases: [{ tag: 'v7.0.0' }, { tag: 'v6.9.9' }],
  }));
  repo.commit('update release records');
  repo.git('push', '--quiet', 'origin', 'main');
  repo.git('checkout', '--quiet', '--detach', repo.base);
  assertDeploy(repo.run(['current']), true);

  repo.git('checkout', '--quiet', 'main');
  repo.writeCatalog(['7.0.1', '7.0.0']);
  const newCatalog = repo.commit('prepare a new release');
  repo.git('push', '--quiet', 'origin', 'main');
  repo.git('checkout', '--quiet', '--detach', repo.base);
  const staleCatalog = repo.run(['current']);
  assertDeploy(staleCatalog, false);
  assert.match(staleCatalog.stdout, /website inputs have changed/);

  repo.git('checkout', '--quiet', 'main');
  repo.write('apps/website/content/guide.md', 'Updated guide');
  repo.commit('update just the site');
  repo.git('push', '--quiet', 'origin', 'main');
  assertDeploy(repo.run(['current']), true);
  repo.git('checkout', '--quiet', '--detach', newCatalog);
  assertDeploy(repo.run(['current']), false);
});

test('the standard deploy uses the admission results and the release deploy uploads packages first', () => {
  const website = fs.readFileSync(path.join(repository, '.github/workflows/website.yml'), 'utf8');
  const [build, deploy] = website.split('\n  deploy:\n');
  assert.match(build, /deploy: \$\{\{ steps\.publication\.outputs\.deploy \}\}/);
  assert.match(build, /WEBSITE_BEFORE: \$\{\{ github\.event\.before \}\}/);
  assert.match(deploy, /if: github\.ref == 'refs\/heads\/main' && needs\.build\.outputs\.deploy == 'true'/);
  assert.match(deploy, /Check that the build still matches main\n\s+id: current\n\s+run: .*website-deployment\.mjs current/);
  assert.match(deploy, /Upload static website to hiroute-ai\n\s+if: steps\.current\.outputs\.deploy == 'true'/);

  const release = fs.readFileSync(path.join(repository, '.github/workflows/release.yml'), 'utf8');
  assert.match(release, /ref: main/);
  assert.match(release, /ref: \$\{\{ needs\.verify-release\.outputs\.website_revision \}\}/);
  assert.match(release, /publish-release-oss\.sh [^\n]+\n\s+\.github\/scripts\/deploy-website-oss\.sh /);
});
