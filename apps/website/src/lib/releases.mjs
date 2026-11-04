import manifest from '../../data/releases.json' with { type: 'json' };
import shortDownloadVersions from '../../data/short-downloads.json' with { type: 'json' };
import windowsAcceptance from '../../../../scripts/windows-desktop-cases.json' with { type: 'json' };

export const RELEASE_SCHEMA = 'hiroute.website.releases/v3';
export const LEGACY_RELEASE_SCHEMA = 'hiroute.website.releases/v2';
export const DOWNLOAD_ORIGIN = 'https://hiroute.ai';
const SHA256 = /^[0-9a-f]{64}$/;
const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/;
const REVISION = /^[0-9a-f]{40}$/;
const FILENAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,239}$/;
const SHORT_DOWNLOAD_VERSIONS = new Set(shortDownloadVersions);
const STANDALONE_TARGETS = {
  Linux: { x86_64: 'x86_64-unknown-linux-gnu', aarch64: 'aarch64-unknown-linux-gnu' },
};

function requiredText(value, field) {
  if (typeof value !== 'string' || !value.trim()) throw new Error(`${field} must be nonempty text`);
}

export function validateReleaseManifest(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || ![RELEASE_SCHEMA, LEGACY_RELEASE_SCHEMA].includes(value.schema) || !Array.isArray(value.releases) || value.releases.length > 64
      || Object.keys(value).some(key => !['schema', 'releases'].includes(key))) {
    throw new Error('release manifest shape is invalid');
  }
  const versions = new Set();
  let previous = Number.POSITIVE_INFINITY;
  for (const release of value.releases) {
    if (!release || typeof release !== 'object' || Array.isArray(release)
        || Object.keys(release).some(key => !['version', 'channel', 'published_at', 'notes', 'artifacts'].includes(key))
        || !VERSION.test(release.version ?? '') || !['stable', 'preview'].includes(release.channel)
        || !release.notes || typeof release.notes !== 'object'
        || !Array.isArray(release.artifacts) || release.artifacts.length === 0) {
      throw new Error(`invalid release record: ${release?.version ?? 'unknown'}`);
    }
    requiredText(release.notes.zh, 'notes.zh'); requiredText(release.notes.en, 'notes.en');
    const published = Date.parse(release.published_at);
    if (!Number.isFinite(published) || published > previous) throw new Error('releases must use valid newest-first dates');
    previous = published;
    if (versions.has(release.version)) throw new Error(`duplicate release version: ${release.version}`);
    versions.add(release.version);
    const filenames = new Set();
    for (const artifact of release.artifacts) {
      const commonValid = artifact && typeof artifact === 'object' && !Array.isArray(artifact)
        && FILENAME.test(artifact.filename ?? '') && SHA256.test(artifact.sha256 ?? '')
        && Number.isSafeInteger(artifact.size) && artifact.size > 0 && artifact.size <= 2 * 1024 ** 3;
      const desktopValid = commonValid && artifact.kind === 'desktop'
        && !Object.keys(artifact).some(key => !['kind', 'platform', 'architecture', 'format', 'minimum_os', 'distribution', 'filename', 'sha256', 'size'].includes(key))
        && artifact.platform === 'macOS' && ['arm64', 'x86_64'].includes(artifact.architecture)
        && artifact.format === 'dmg' && ['self-signed', 'developer-id'].includes(artifact.distribution)
        && artifact.minimum_os === '15.0';
      const standaloneValid = commonValid && artifact.kind === 'standalone'
        && !Object.keys(artifact).some(key => !['kind', 'platform', 'architecture', 'target', 'format', 'distribution', 'filename', 'sha256', 'size', 'manifest_filename', 'manifest_sha256', 'manifest_size'].includes(key))
        && Object.hasOwn(STANDALONE_TARGETS, artifact.platform)
        && Object.hasOwn(STANDALONE_TARGETS[artifact.platform], artifact.architecture)
        && artifact.target === STANDALONE_TARGETS[artifact.platform][artifact.architecture]
        && artifact.format === 'tar.gz' && artifact.distribution === 'unsigned'
        && artifact.manifest_filename === `${artifact.filename}.json`
        && FILENAME.test(artifact.manifest_filename ?? '') && SHA256.test(artifact.manifest_sha256 ?? '')
        && Number.isSafeInteger(artifact.manifest_size) && artifact.manifest_size > 0;
      const windowsDesktopValid = commonValid && value.schema === RELEASE_SCHEMA && artifact.kind === 'desktop'
        && !Object.keys(artifact).some(key => !['kind', 'platform', 'architecture', 'format', 'distribution', 'filename', 'sha256', 'size'].includes(key))
        && artifact.platform === 'Windows' && artifact.architecture === 'x86_64' && artifact.format === 'nsis' && artifact.distribution === 'unsigned';
      if (!desktopValid && !windowsDesktopValid && !standaloneValid) {
        throw new Error(`invalid public artifact in ${release.version}`);
      }
      if (desktopValid) {
        const suffix = artifact.distribution === 'self-signed' ? 'trial' : 'developer-id';
        const expected = new RegExp(`^HiRoute-${release.version.replaceAll('.', '\\.')}-[0-9a-f]{12}-macos-${artifact.architecture}-${suffix}\\.dmg$`);
        if (!expected.test(artifact.filename)) throw new Error(`artifact filename does not match release: ${artifact.filename}`);
      }
      if (windowsDesktopValid) {
        const prefix = `HiRoute-${release.version}-`, suffix = '-windows-x86_64-setup.exe';
        const revision = artifact.filename.startsWith(prefix) && artifact.filename.endsWith(suffix)
          ? artifact.filename.slice(prefix.length, -suffix.length) : '';
        if (!/^[0-9a-f]{12}$/.test(revision)) throw new Error(`artifact filename does not match release: ${artifact.filename}`);
      }
      if (standaloneValid) {
        const expected = new RegExp(`^hiroute-${release.version.replaceAll('.', '\\.')}-[0-9a-f]{12}-${artifact.target}\\.tar\\.gz$`);
        if (!expected.test(artifact.filename)) throw new Error(`artifact filename does not match release: ${artifact.filename}`);
      }
      if (filenames.has(artifact.filename)) throw new Error(`duplicate artifact: ${artifact.filename}`);
      filenames.add(artifact.filename);
      if (standaloneValid) {
        if (filenames.has(artifact.manifest_filename)) throw new Error(`duplicate artifact: ${artifact.manifest_filename}`);
        filenames.add(artifact.manifest_filename);
      }
    }
    if (SHORT_DOWNLOAD_VERSIONS.has(release.version)) {
      for (const artifact of release.artifacts) {
        const name = shortDownloadName(release, artifact);
        for (const filename of artifact.kind === 'standalone' ? [name, `${name}.json`] : [name]) {
          if (!FILENAME.test(filename) || filenames.has(filename)) throw new Error(`invalid or duplicate short download filename: ${filename}`);
          filenames.add(filename);
        }
      }
    }
  }
  return value;
}

export function projectReleaseManifest(value, schema = RELEASE_SCHEMA) {
  validateReleaseManifest(value);
  if (![RELEASE_SCHEMA, LEGACY_RELEASE_SCHEMA].includes(schema)) throw new Error('unsupported catalog projection');
  const projected = structuredClone(value); projected.schema = schema;
  if (schema === LEGACY_RELEASE_SCHEMA) {
    projected.releases = projected.releases.flatMap(release => {
      release.artifacts = release.artifacts.filter(artifact => artifact.platform !== 'Windows');
      return release.artifacts.length ? [release] : [];
    });
  }
  return validateReleaseManifest(projected);
}

function releaseKey(release, filename) {
  return `releases/${release.version}/${filename}`;
}

export function artifactKey(release, artifact) {
  return releaseKey(release, artifact.filename);
}

function shortDownloadName(release, artifact) {
  const name = `HiRoute-${release.version}`;
  if (artifact.platform === 'Windows') return `${name}.exe`;
  if (artifact.platform === 'macOS') return `${name}-macos-${artifact.architecture}.dmg`;
  return `${name}-linux-${artifact.architecture}.tar.gz`;
}

export function artifactUrl(release, artifact) {
  const filename = SHORT_DOWNLOAD_VERSIONS.has(release.version) ? shortDownloadName(release, artifact) : artifact.filename;
  return `${DOWNLOAD_ORIGIN}/${releaseKey(release, filename)}`;
}

export function artifactManifestKey(release, artifact) {
  if (artifact.kind !== 'standalone') throw new Error('only standalone artifacts have package manifests');
  return releaseKey(release, artifact.manifest_filename);
}

export function artifactManifestUrl(release, artifact) {
  const originalKey = artifactManifestKey(release, artifact);
  const key = SHORT_DOWNLOAD_VERSIONS.has(release.version) ? releaseKey(release, `${shortDownloadName(release, artifact)}.json`) : originalKey;
  return `${DOWNLOAD_ORIGIN}/${key}`;
}

export function artifactAssets(release, artifact) {
  const assets = [{
    filename: artifact.filename,
    key: artifactKey(release, artifact),
    sha256: artifact.sha256,
    size: artifact.size,
  }];
  if (artifact.kind === 'standalone') assets.push({
    filename: artifact.manifest_filename,
    key: artifactManifestKey(release, artifact),
    sha256: artifact.manifest_sha256,
    size: artifact.manifest_size,
  });
  return assets;
}

export function publicationAssets(release, artifact) {
  const assets = artifactAssets(release, artifact);
  if (!SHORT_DOWNLOAD_VERSIONS.has(release.version)) return assets;
  const name = shortDownloadName(release, artifact);
  const aliases = assets.map((asset, index) => ({
    ...asset,
    key: releaseKey(release, index === 0 ? name : `${name}.json`),
  }));
  return [...assets, ...aliases];
}

export function verifyWindowsNativeAcceptance(identity, policy = windowsAcceptance) {
  if (policy.schema !== 'hiroute.windows-desktop-acceptance/v1') throw new Error('unknown Windows native acceptance policy');
  const cases = policy.platforms;
  const runs = identity.native_evidence?.runs;
  if (!Array.isArray(runs) || runs.length !== Object.keys(cases).length
      || new Set(runs.map(run => run?.platform)).size !== runs.length
      || runs.some(run => !cases[run?.platform])) throw new Error('Windows Desktop native selections are incomplete');
  let deferred = Object.fromEntries(Object.keys(cases).map(platform => [platform, []]));
  if (identity.native_runtime !== 'green') {
    const decisions = policy.release_decisions.filter(entry => ['version', 'revision', 'installer_sha256'].every(key => entry[key] === identity[key]));
    if (identity.native_runtime !== 'deferred' || decisions.length !== 1
        || identity.acceptance_decision !== decisions[0].decision_url
        || !/^https:\/\/github\.com\/higress-group\/HiRoute\/issues\/[1-9][0-9]*#issuecomment-[1-9][0-9]*$/.test(decisions[0].decision_url)) {
      throw new Error('Windows native deferral lacks an exact committed release decision');
    }
    deferred = decisions[0].deferred_cases ?? {};
    if (Object.keys(deferred).length !== Object.keys(cases).length || Object.keys(cases).some(key => !Object.hasOwn(deferred, key))
        || !Object.values(deferred).some(list => Array.isArray(list) && list.length)
        || Object.entries(deferred).some(([platform, list]) => !Array.isArray(list) || new Set(list).size !== list.length
          || list.some(id => !cases[platform]?.includes(id) || ['install.standard-user', 'install.console-ready'].includes(id)))) {
      throw new Error('native deferral cannot omit actual standard-user installation and console readiness');
    }
  }
  if (runs.some(run => run.revision !== identity.revision || run.installer_sha256 !== identity.installer_sha256
      || run.standard_user !== true || run.architecture !== 'AMD64' || !run.os_version
      || !run.scenarios || Array.isArray(run.scenarios) || Object.keys(run.scenarios).length !== cases[run.platform].length
      || cases[run.platform].some(id => run.scenarios[id] !== (deferred[run.platform].includes(id) ? 'not_assessed' : 'green')))) {
    throw new Error('Windows Desktop native acceptance is incomplete or belongs to another artifact');
  }
  return identity;
}

export function verifyDesktopIdentity(release, artifact, identity, expectedRevision) {
  if (artifact.kind !== 'desktop' || !REVISION.test(expectedRevision ?? '')) {
    throw new Error('desktop identity verification inputs are invalid');
  }
  if (artifact.platform === 'Windows') {
    const expected = { schema: 'hiroute.windows-desktop-package/v1', version: release.version, revision: expectedRevision,
      architecture: 'x86_64', distribution: 'unsigned', installer_sha256: artifact.sha256, installer_size: artifact.size, integrity: 'green', embedded_components: 'green' };
    if (!identity || typeof identity !== 'object' || Object.entries(expected).some(([key, value]) => identity[key] !== value)
        || artifact.filename !== `HiRoute-${release.version}-${expectedRevision.slice(0, 12)}-windows-x86_64-setup.exe`) throw new Error('Windows Desktop identity or native acceptance differs from the release');
    return verifyWindowsNativeAcceptance(identity);
  }
  const expectedDistribution = artifact.distribution === 'self-signed' ? 'controlled-trial' : 'developer-id';
  const expected = {
    version: release.version,
    revision: expectedRevision,
    architecture: artifact.architecture,
    distribution: expectedDistribution,
    dmg_sha256: artifact.sha256,
    integrity: 'green',
    mounted_components: 'green',
    detach: 'green',
  };
  if (!identity || typeof identity !== 'object' || Array.isArray(identity)
      || Object.entries(expected).some(([key, value]) => identity[key] !== value)) {
    throw new Error(`desktop package identity differs from release record: ${artifact.filename}`);
  }
  const suffix = artifact.distribution === 'self-signed' ? 'trial' : 'developer-id';
  const expectedFilename = `HiRoute-${release.version}-${expectedRevision.slice(0, 12)}-macos-${artifact.architecture}-${suffix}.dmg`;
  if (artifact.filename !== expectedFilename) {
    throw new Error(`desktop artifact filename differs from release tag: ${artifact.filename}`);
  }
  return identity;
}

export function latestRelease(channel = 'stable', value = manifest) {
  return validateReleaseManifest(value).releases.find(release => release.channel === channel) ?? null;
}

export const releaseManifest = validateReleaseManifest(manifest);

export function formatBytes(bytes, language = 'zh') {
  const units = language === 'zh' ? ['B', 'KB', 'MB', 'GB'] : ['B', 'KB', 'MB', 'GB'];
  let value = bytes, index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
  return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}
