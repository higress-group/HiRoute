import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import {
  artifactAssets, artifactKey, artifactManifestUrl, artifactUrl, latestRelease, publicationAssets, validateReleaseManifest,
  verifyDesktopIdentity,
  projectReleaseManifest, LEGACY_RELEASE_SCHEMA,
  verifyWindowsNativeAcceptance,
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

test('new Windows installers require complete standard-user Server evidence on the exact artifact', () => {
  const binding = { version: '1.2.3', revision, installer_sha256: 'c'.repeat(64) };
  const run = { ...binding, platform: 'server', standard_user: true, architecture: 'AMD64',
    os_version: 'Windows-2025Server-10.0.26100-SP0',
    scenarios: { 'install.standard-user': 'green', 'install.console-ready': 'green',
      'console.source-plan-agent': 'green', 'uninstall.restore-agent': 'green' } };
  const identity = { ...binding, native_runtime: 'green', native_evidence: { runs: [run] } };
  assert.equal(verifyWindowsNativeAcceptance(identity), identity);
  for (const override of [
    { installer_sha256: 'd'.repeat(64) }, { revision: 'd'.repeat(40) }, { standard_user: false },
    { architecture: 'ARM64' }, { os_version: 'Windows-11-10.0.26100-SP0' },
    { scenarios: { ...run.scenarios, 'uninstall.restore-agent': 'not_assessed' } },
    { scenarios: { ...run.scenarios, 'uninstall.restore-agent': undefined } },
  ]) {
    assert.throws(() => verifyWindowsNativeAcceptance({ ...identity, native_evidence: { runs: [{ ...run, ...override }] } }), /incomplete/);
  }
  assert.throws(() => verifyWindowsNativeAcceptance({ ...identity, native_evidence: { runs: [run, { ...run, platform: 'windows11' }] } }), /incomplete/);
  identity.native_runtime = 'deferred';
  assert.throws(() => verifyWindowsNativeAcceptance(identity), /exact committed release decision/);
});

test('historical Windows identities retain the exact recorded selection and verdicts', () => {
  const policy = JSON.parse(fs.readFileSync(new URL('../../../scripts/windows-desktop-cases.json', import.meta.url)));
  const decision = policy.release_decisions.find(item => item.version === '0.6.0');
  const { version, revision, installer_sha256 } = decision;
  const binding = { version, revision, installer_sha256 };
  const runs = Object.entries(policy.historical_platforms).map(([platform, cases]) => ({
    ...binding, platform, standard_user: true, architecture: 'AMD64', os_version: 'Windows-2025Server-10.0.26100-SP0',
    scenarios: Object.fromEntries(cases.map(id => [id, decision.deferred_cases[platform].includes(id) ? 'not_assessed' : 'green'])),
  }));
  const identity = { ...binding, native_runtime: 'deferred', acceptance_decision: decision.decision_url,
    native_evidence: { runs } };
  assert.equal(verifyWindowsNativeAcceptance(identity, policy), identity);
  for (const mutate of [
    value => { value.revision = 'b'.repeat(40); },
    value => { value.version = '0.6.1'; },
    value => { value.installer_sha256 = 'd'.repeat(64); },
    value => { value.acceptance_decision += '0'; },
    value => { value.native_evidence.runs[0].standard_user = false; },
    value => { value.native_evidence.runs[0].scenarios['update.killed-resume'] = 'failed'; },
    value => { value.native_evidence.runs[0].scenarios['update.killed-resume'] = 'green'; },
    value => { delete value.native_evidence.runs[0].scenarios['update.killed-resume']; },
    value => { value.native_evidence.runs[0].scenarios['unregistered-case'] = 'green'; },
  ]) {
    const altered = structuredClone(identity); mutate(altered);
    assert.throws(() => verifyWindowsNativeAcceptance(altered, policy));
  }
  assert.throws(() => verifyWindowsNativeAcceptance({ ...binding, native_runtime: 'green', native_evidence: { runs: runs.slice(1) } }, policy),
    /exact committed release decision/);
  decision.deferred_cases.windows11.push('install.console-ready');
  identity.native_evidence.runs[0].scenarios['install.console-ready'] = 'not_assessed';
  assert.throws(() => verifyWindowsNativeAcceptance(identity, policy), /cannot omit actual standard-user/);
});
test('Desktop update endpoint publishes the same validated website catalog', () => {
  const generated = JSON.parse(fs.readFileSync(new URL('../public/releases.json', import.meta.url), 'utf8'));
  const source = JSON.parse(fs.readFileSync(new URL('../data/releases.json', import.meta.url), 'utf8'));
  assert.deepEqual(generated, projectReleaseManifest(source, LEGACY_RELEASE_SCHEMA));
  const current = JSON.parse(fs.readFileSync(new URL('../public/releases-v3.json', import.meta.url), 'utf8'));
  assert.deepEqual(current, projectReleaseManifest(source));
});

test('Windows Desktop uses V3 and can never enter the released macOS V2 projection', () => {
  const value = structuredClone(manifest); value.schema = 'hiroute.website.releases/v3';
  const windows = { kind: 'desktop', platform: 'Windows', architecture: 'x86_64', format: 'nsis', distribution: 'unsigned',
    filename: 'HiRoute-1.2.3-aaaaaaaaaaaa-windows-x86_64-setup.exe', sha256: 'c'.repeat(64), size: 512 };
  value.releases[0].artifacts.push(windows);
  assert.equal(validateReleaseManifest(value), value);
  assert.deepEqual(projectReleaseManifest(value, LEGACY_RELEASE_SCHEMA), manifest);
  for (const [field, entry] of [['architecture', 'aarch64'], ['format', 'exe'], ['distribution', 'self-signed'], ['minimum_os', 'any'], ['filename', '../setup.exe']]) {
    const invalid = structuredClone(value); invalid.releases[0].artifacts[1][field] = entry;
    assert.throws(() => validateReleaseManifest(invalid), field);
  }
  const onlyWindows = structuredClone(value); onlyWindows.releases[0].artifacts = [windows];
  assert.equal(projectReleaseManifest(onlyWindows, LEGACY_RELEASE_SCHEMA).releases.length, 0);
});
const standalone = {
  kind: 'standalone', platform: 'Linux', architecture: 'x86_64', target: 'x86_64-unknown-linux-gnu',
  format: 'tar.gz', distribution: 'unsigned',
  filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz', sha256: 'c'.repeat(64), size: 456789,
  manifest_filename: 'hiroute-1.2.3-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz.json',
  manifest_sha256: 'd'.repeat(64), manifest_size: 3456,
};

test('Omarchy Desktop publishes signed companion files while preserving both updater feeds', () => {
  const value = structuredClone(manifest);
  value.schema = 'hiroute.website.releases/v3';
  const current = value.releases[0];
  current.version = '0.5.1';
  current.artifacts[0].filename = artifact.filename.replace('1.2.3', current.version);
  const original = structuredClone(value);
  const filename = 'HiRoute-0.5.1-aaaaaaaaaaaa-omarchy-x86_64.pkg.tar.xz';
  const omarchy = {
    kind: 'desktop', platform: 'Omarchy', architecture: 'x86_64', format: 'pkg.tar.xz', distribution: 'openpgp',
    filename, sha256: 'a'.repeat(64), size: 1024,
    signature_filename: `${filename}.sig`, signature_sha256: 'b'.repeat(64), signature_size: 128,
    checksum_filename: `${filename}.sha256`, checksum_sha256: 'c'.repeat(64), checksum_size: 140,
  };
  current.artifacts.push(omarchy);
  assert.equal(validateReleaseManifest(value), value);
  assert.deepEqual(projectReleaseManifest(value), original);
  assert.deepEqual(projectReleaseManifest(value, LEGACY_RELEASE_SCHEMA), projectReleaseManifest(original, LEGACY_RELEASE_SCHEMA));
  assert.equal(artifactUrl(current, omarchy), `https://hiroute.ai/releases/0.5.1/${filename}`);
  assert.deepEqual(publicationAssets(current, omarchy).map(item => [item.filename, item.key]),
    [filename, omarchy.signature_filename, omarchy.checksum_filename].map(name => [name, `releases/0.5.1/${name}`]));
  for (const drift of [
    { architecture: 'arm64' }, { distribution: 'unsigned' }, { signature_filename: 'other.sig' },
    { checksum_sha256: 'invalid' }, { signature_size: 0 },
  ]) {
    const altered = structuredClone(value);
    Object.assign(altered.releases[0].artifacts[1], drift);
    assert.throws(() => validateReleaseManifest(altered));
  }
});

test('release links use the immutable hiroute.ai version path', () => {
  assert.equal(validateReleaseManifest(manifest), manifest);
  assert.equal(artifactKey(release, artifact), `releases/1.2.3/${artifact.filename}`);
  assert.equal(artifactUrl(release, artifact), `https://hiroute.ai/releases/1.2.3/${artifact.filename}`);
  assert.equal(latestRelease('stable', manifest), release);
});

test('short download links preserve the exact package bytes and native update catalog', () => {
  const value = structuredClone(manifest);
  value.schema = 'hiroute.website.releases/v3';
  const current = value.releases[0];
  current.version = '0.5.1';
  const windows = { kind: 'desktop', platform: 'Windows', architecture: 'x86_64', format: 'nsis',
    distribution: 'unsigned', filename: 'HiRoute-0.5.1-aaaaaaaaaaaa-windows-x86_64-setup.exe', sha256: 'c'.repeat(64), size: 512 };
  current.artifacts = [windows, {
    ...artifact, filename: artifact.filename.replace('1.2.3', current.version),
  }, {
    ...standalone, filename: standalone.filename.replace('1.2.3', current.version),
    manifest_filename: standalone.manifest_filename.replace('1.2.3', current.version),
  }];
  assert.equal(validateReleaseManifest(value), value);
  assert.deepEqual(projectReleaseManifest(value), value);
  assert.deepEqual(projectReleaseManifest(value, LEGACY_RELEASE_SCHEMA).releases[0].artifacts, current.artifacts.slice(1));
  const names = ['HiRoute-0.5.1.exe', 'HiRoute-0.5.1-macos-arm64.dmg', 'HiRoute-0.5.1-linux-x86_64.tar.gz'];
  for (const [index, row] of current.artifacts.entries()) {
    const key = `releases/0.5.1/${names[index]}`;
    assert.equal(artifactUrl(current, row), `https://hiroute.ai/${key}`);
    const canonical = artifactAssets(current, row);
    const aliasKeys = row.kind === 'standalone' ? [key, `${key}.json`] : [key];
    assert.deepEqual(publicationAssets(current, row), [
      ...canonical, ...canonical.map((entry, n) => ({ ...entry, key: aliasKeys[n] })),
    ]);
  }
  assert.equal(artifactManifestUrl(current, current.artifacts[2]), `https://hiroute.ai/releases/0.5.1/${names[2]}.json`);
  current.artifacts.push({
    ...current.artifacts[1], distribution: 'developer-id',
    filename: current.artifacts[1].filename.replace('-trial.dmg', '-developer-id.dmg'),
  });
  assert.throws(() => validateReleaseManifest(value), /duplicate short download filename/);
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

test('one release accepts both Mac architectures and both Linux architectures', () => {
  const intel = { ...artifact, architecture: 'x86_64', filename: artifact.filename.replace('arm64', 'x86_64') };
  const arm = { ...standalone, architecture: 'aarch64', target: 'aarch64-unknown-linux-gnu',
    filename: standalone.filename.replace('x86_64', 'aarch64'),
    manifest_filename: standalone.manifest_filename.replace('x86_64', 'aarch64') };
  const value = structuredClone(manifest);
  value.releases[0].artifacts = [artifact, intel, standalone, arm];
  assert.equal(validateReleaseManifest(value), value);
  assert.equal(value.releases[0].artifacts.flatMap(a => artifactAssets(value.releases[0], a)).length, 6);
  const identity = { version: release.version, revision, architecture: 'x86_64',
    distribution: 'controlled-trial', dmg_sha256: intel.sha256,
    integrity: 'green', mounted_components: 'green', detach: 'green' };
  assert.equal(verifyDesktopIdentity(value.releases[0], intel, identity, revision), identity);
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

test('Windows standalone packages are rejected for both architectures', () => {
  for (const architecture of ['x86_64', 'aarch64']) {
    const target = `${architecture}-pc-windows-msvc`;
    const candidate = { ...standalone, platform: 'Windows', architecture, target,
      filename: `hiroute-1.2.3-aaaaaaaaaaaa-${target}.tar.gz`,
      manifest_filename: `hiroute-1.2.3-aaaaaaaaaaaa-${target}.tar.gz.json` };
    const value = { ...structuredClone(manifest), releases: [{ ...structuredClone(release), artifacts: [candidate] }] };
    assert.throws(() => validateReleaseManifest(value), /invalid public artifact/);
    for (const drift of [
      { platform: 'Linux' }, { architecture: architecture === 'aarch64' ? 'x86_64' : 'aarch64' },
      { target: `${architecture}-unknown-linux-gnu` }, { target: `${architecture}-pc-windows-gnu` },
    ]) {
      const invalid = structuredClone(value);
      Object.assign(invalid.releases[0].artifacts[0], drift);
      assert.throws(() => validateReleaseManifest(invalid));
    }
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
