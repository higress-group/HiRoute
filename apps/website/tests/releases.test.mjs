import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import {
  artifactAssets, artifactKey, artifactManifestUrl, artifactUrl, latestRelease, validateReleaseManifest,
  verifyDesktopIdentity,
} from '../src/lib/releases.mjs';

const revision = 'a'.repeat(40);

const artifact = {
  kind: 'desktop', platform: 'macOS', architecture: 'arm64', format: 'dmg', minimum_os: '15.0',
  distribution: 'self-signed', filename: 'HiRoute-1.2.3-aaaaaaaaaaaa-macos-arm64-trial.dmg',
  sha256: 'b'.repeat(64), size: 123456,
};
const release = {
  version: '1.2.3', channel: 'stable', published_at: '2026-09-21T00:00:00Z',
  notes: { zh: '正式版本。', en: 'Stable release.' }, artifacts: [artifact],
};
const manifest = { schema: 'hiroute.website.releases/v2', releases: [release] };
test('Desktop update endpoint publishes the same validated website catalog', () => {
  const generated = JSON.parse(fs.readFileSync(new URL('../public/releases.json', import.meta.url), 'utf8'));
  const source = JSON.parse(fs.readFileSync(new URL('../data/releases.json', import.meta.url), 'utf8'));
  assert.deepEqual(generated, validateReleaseManifest(source));
});
const standalone = {
  kind: 'standalone', platform: 'Linux', architecture: 'x86_64', target: 'x86_64-unknown-linux-gnu',
  format: 'tar.gz', distribution: 'unsigned',
  filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz', sha256: 'c'.repeat(64), size: 456789,
  manifest_filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz.json',
  manifest_sha256: 'd'.repeat(64), manifest_size: 3456,
};

test('release links use the immutable hiroute.ai version path', () => {
  assert.equal(validateReleaseManifest(manifest), manifest);
  assert.equal(artifactKey(release, artifact), `releases/1.2.3/${artifact.filename}`);
  assert.equal(artifactUrl(release, artifact), `https://hiroute.ai/releases/1.2.3/${artifact.filename}`);
  assert.equal(latestRelease('stable', manifest), release);
});

test('public release manifest accepts declared self-signed and developer-id artifacts', () => {
  assert.equal(validateReleaseManifest(manifest), manifest);
  const signed = structuredClone(manifest);
  signed.releases[0].artifacts[0].distribution = 'developer-id';
  signed.releases[0].artifacts[0].filename = signed.releases[0].artifacts[0].filename.replace('trial', 'developer-id');
  assert.equal(validateReleaseManifest(signed), signed);
});

test('standalone artifacts publish an archive and companion package manifest', () => {
  const value = structuredClone(manifest);
  value.releases[0].artifacts.push(standalone);
  assert.equal(validateReleaseManifest(value), value);
  assert.equal(artifactManifestUrl(value.releases[0], standalone),
    `https://hiroute.ai/releases/1.2.3/${standalone.manifest_filename}`);
  assert.deepEqual(artifactAssets(value.releases[0], standalone).map(asset => asset.filename),
    [standalone.filename, standalone.manifest_filename]);
});

test('public release manifest rejects mismatched distribution, duplicate and unsafe artifacts', () => {
  const copy = value => structuredClone(value);
  for (const mutate of [
    value => { value.releases[0].revision = revision; },
    value => { value.releases[0].artifacts[0].distribution = 'developer-id'; },
    value => { value.releases[0].artifacts[0].distribution = 'unknown'; },
    value => { value.releases[0].artifacts[0].filename = '../HiRoute.dmg'; },
    value => { value.releases[0].artifacts[0].sha256 = 'not-a-digest'; },
    value => { value.releases.push(copy(value.releases[0])); },
  ]) {
    const invalid = copy(manifest); mutate(invalid);
    assert.throws(() => validateReleaseManifest(invalid));
  }
});

test('standalone artifacts reject target, filename, and companion-manifest drift', () => {
  for (const mutate of [
    value => { value.target = 'aarch64-unknown-linux-gnu'; },
    value => { value.filename = value.filename.replace('x86_64-unknown-linux-gnu', 'wrong-target'); },
    value => { value.manifest_filename = 'manifest.json'; },
    value => { value.manifest_sha256 = 'not-a-digest'; },
  ]) {
    const value = structuredClone(manifest);
    const candidate = structuredClone(standalone);
    mutate(candidate);
    value.releases[0].artifacts.push(candidate);
    assert.throws(() => validateReleaseManifest(value));
  }
});

test('desktop verification binds public metadata to package and tag identities', () => {
  const identity = {
    version: release.version, revision, architecture: artifact.architecture,
    distribution: 'controlled-trial', dmg_sha256: artifact.sha256,
    integrity: 'green', mounted_components: 'green', detach: 'green',
  };
  assert.equal(verifyDesktopIdentity(release, artifact, identity, revision), identity);
  assert.throws(() => verifyDesktopIdentity(release, artifact, { ...identity, architecture: 'x86_64' }, revision),
    /identity differs/);
  assert.throws(() => verifyDesktopIdentity(release, artifact,
    { ...identity, revision: 'b'.repeat(40) }, 'b'.repeat(40)), /filename differs/);

  const signedRelease = structuredClone(release);
  const signed = signedRelease.artifacts[0];
  signed.distribution = 'developer-id';
  signed.filename = signed.filename.replace('trial', 'developer-id');
  assert.doesNotThrow(() => verifyDesktopIdentity(signedRelease, signed,
    { ...identity, distribution: 'developer-id' }, revision));
});

test('release workflow CLI consumes the packager desktop identity report', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-desktop-identity-'));
  const manifestPath = path.join(root, 'releases.json');
  const identityPath = path.join(root, 'identity.json');
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  fs.writeFileSync(identityPath, JSON.stringify({
    version: release.version, revision, architecture: artifact.architecture,
    distribution: 'controlled-trial', dmg_sha256: artifact.sha256,
    integrity: 'green', mounted_components: 'green', detach: 'green',
  }));
  const script = new URL('../scripts/release-manifest.mjs', import.meta.url);
  const result = spawnSync(process.execPath, [script.pathname, 'verify-desktop-identity', manifestPath,
    '--tag', 'v1.2.3', '--revision', revision, '--filename', artifact.filename,
    '--identity', identityPath], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /identity verified/);
});

test('releases are newest first and score no fake empty public package', () => {
  assert.equal(latestRelease('stable', { schema: manifest.schema, releases: [] }), null);
  const invalid = structuredClone(manifest);
  invalid.releases.push({ ...structuredClone(release), version: '1.2.2', published_at: '2026-09-22T00:00:00Z' });
  assert.throws(() => validateReleaseManifest(invalid), /newest-first/);
});

test('release verification binds a standalone package manifest to the website release', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-release-manifest-'));
  const archive = Buffer.from('standalone-archive-fixture');
  const archiveSha = crypto.createHash('sha256').update(archive).digest('hex');
  const candidate = structuredClone(standalone);
  candidate.size = archive.length;
  candidate.sha256 = archiveSha;
  const packageManifest = {
    schema: 'hiroute.standalone-package/v1', version: release.version, revision,
    target: candidate.target, distribution: 'integration-candidate',
    archive: { filename: candidate.filename, sha256: archiveSha, size: archive.length },
  };
  const packageBytes = Buffer.from(`${JSON.stringify(packageManifest)}\n`);
  candidate.manifest_size = packageBytes.length;
  candidate.manifest_sha256 = crypto.createHash('sha256').update(packageBytes).digest('hex');
  fs.writeFileSync(path.join(root, candidate.filename), archive);
  fs.writeFileSync(path.join(root, candidate.manifest_filename), packageBytes);
  const value = structuredClone(manifest);
  value.releases[0].artifacts = [candidate];
  const manifestPath = path.join(root, 'releases.json');
  fs.writeFileSync(manifestPath, JSON.stringify(value));
  const script = new URL('../scripts/release-manifest.mjs', import.meta.url);
  const good = spawnSync(process.execPath, [script.pathname, 'verify-assets', manifestPath, '--tag', 'v1.2.3', '--revision', revision, '--asset-dir', root], { encoding: 'utf8' });
  assert.equal(good.status, 0, good.stderr);
  assert.match(good.stdout, /2 release asset\(s\) verified/);

  packageManifest.version = '9.9.9';
  const drifted = Buffer.from(`${JSON.stringify(packageManifest)}\n`);
  fs.writeFileSync(path.join(root, candidate.manifest_filename), drifted);
  value.releases[0].artifacts[0].manifest_size = drifted.length;
  value.releases[0].artifacts[0].manifest_sha256 = crypto.createHash('sha256').update(drifted).digest('hex');
  fs.writeFileSync(manifestPath, JSON.stringify(value));
  const bad = spawnSync(process.execPath, [script.pathname, 'verify-assets', manifestPath, '--tag', 'v1.2.3', '--revision', revision, '--asset-dir', root], { encoding: 'utf8' });
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /package identity differs from release record/);
});

test('release verification rejects an artifact filename from a different tag revision', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'hiroute-release-tag-'));
  const manifestPath = path.join(root, 'releases.json');
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  const script = new URL('../scripts/release-manifest.mjs', import.meta.url);
  const result = spawnSync(process.execPath, [script.pathname, 'verify-assets', manifestPath,
    '--tag', 'v1.2.3', '--revision', 'b'.repeat(40), '--asset-dir', root], { encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /filename differs from release tag/);
});
