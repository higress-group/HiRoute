import manifest from '../../data/releases.json' with { type: 'json' };
import windowsDesktopCases from '../../../../scripts/windows-desktop-cases.json' with { type: 'json' };

export const RELEASE_SCHEMA = 'hiroute.website.releases/v3';
export const LEGACY_RELEASE_SCHEMA = 'hiroute.website.releases/v2';
export const DOWNLOAD_ORIGIN = 'https://hiroute.ai';
const SHA256 = /^[0-9a-f]{64}$/;
const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/;
const REVISION = /^[0-9a-f]{40}$/;
const FILENAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,239}$/;
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

export function artifactKey(release, artifact) {
  return `releases/${release.version}/${artifact.filename}`;
}

export function artifactUrl(release, artifact) {
  return `${DOWNLOAD_ORIGIN}/${artifactKey(release, artifact)}`;
}

export function artifactManifestKey(release, artifact) {
  if (artifact.kind !== 'standalone') throw new Error('only standalone artifacts have package manifests');
  return `releases/${release.version}/${artifact.manifest_filename}`;
}

export function artifactManifestUrl(release, artifact) {
  return `${DOWNLOAD_ORIGIN}/${artifactManifestKey(release, artifact)}`;
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

export function verifyDesktopIdentity(release, artifact, identity, expectedRevision) {
  if (artifact.kind !== 'desktop' || !REVISION.test(expectedRevision ?? '')) {
    throw new Error('desktop identity verification inputs are invalid');
  }
  if (artifact.platform === 'Windows') {
    const expected = { schema: 'hiroute.windows-desktop-package/v1', version: release.version, revision: expectedRevision,
      architecture: 'x86_64', distribution: 'unsigned', installer_sha256: artifact.sha256, integrity: 'green', embedded_components: 'green', native_runtime: 'green' };
    if (!identity || typeof identity !== 'object' || Object.entries(expected).some(([key, value]) => identity[key] !== value)
        || artifact.filename !== `HiRoute-${release.version}-${expectedRevision.slice(0, 12)}-windows-x86_64-setup.exe`) throw new Error('Windows Desktop identity or native acceptance differs from the release');
    const runs = identity.native_evidence?.runs;
    if (!Array.isArray(runs) || runs.length !== Object.keys(windowsDesktopCases).length
        || new Set(runs.map(run => run?.platform)).size !== runs.length
        || runs.some(run => !windowsDesktopCases[run?.platform] || run.revision !== expectedRevision
          || run.installer_sha256 !== artifact.sha256 || run.standard_user !== true || run.architecture !== 'AMD64' || !run.os_version
          || windowsDesktopCases[run.platform].some(id => run.scenarios?.[id] !== 'green')
          || Object.values(run.scenarios ?? {}).some(value => value !== 'green'))) throw new Error('Windows Desktop native acceptance is incomplete or belongs to another artifact');
    return identity;
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
