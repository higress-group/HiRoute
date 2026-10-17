#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { artifactAssets, validateReleaseManifest, verifyDesktopIdentity } from '../src/lib/releases.mjs';

const [command, manifestPath, ...args] = process.argv.slice(2);
if (!command || !manifestPath) throw new Error('usage: release-manifest.mjs <validate|artifacts|desktop-artifacts|standalone-packages|verify-assets|verify-desktop-identity> MANIFEST [options]');
const manifest = validateReleaseManifest(JSON.parse(fs.readFileSync(manifestPath, 'utf8')));
const tagIndex = args.indexOf('--tag');
const tag = tagIndex >= 0 ? args[tagIndex + 1] : null;
const release = tag ? manifest.releases.find(item => `v${item.version}` === tag) : null;
if (tag && !release) throw new Error(`no release manifest record matches ${tag}`);
const revisionIndex = args.indexOf('--revision');
const revision = revisionIndex >= 0 ? args[revisionIndex + 1] : null;
if (revision !== null && !/^[0-9a-f]{40}$/.test(revision)) throw new Error('--revision must be a full lowercase commit SHA');
const channelIndex = args.indexOf('--channel');
if (release && channelIndex >= 0 && release.channel !== args[channelIndex + 1]) throw new Error('release channel differs from GitHub release');

function expectedFilename(artifact) {
  if (!release || !revision) throw new Error('--tag and --revision are required');
  if (artifact.kind === 'desktop') {
    if (artifact.platform === 'Windows') return `HiRoute-${release.version}-${revision.slice(0, 12)}-windows-x86_64-setup.exe`;
    const suffix = artifact.distribution === 'self-signed' ? 'trial' : 'developer-id';
    return `HiRoute-${release.version}-${revision.slice(0, 12)}-macos-${artifact.architecture}-${suffix}.dmg`;
  }
  return `hiroute-${release.version}-${revision.slice(0, 12)}-${artifact.target}.tar.gz`;
}

if (command === 'validate') {
  process.stdout.write(`${manifest.releases.length} release(s) valid\n`);
} else if (command === 'artifacts') {
  const selected = release ? [release] : manifest.releases;
  for (const item of selected) {
    for (const artifact of item.artifacts) {
      for (const asset of artifactAssets(item, artifact)) {
        process.stdout.write([asset.filename, asset.key, asset.sha256, asset.size].join('\t') + '\n');
      }
    }
  }
} else if (command === 'desktop-artifacts') {
  if (!release) throw new Error('--tag is required');
  for (const artifact of release.artifacts.filter(item => item.kind === 'desktop')) {
    process.stdout.write(`${artifact.filename}\n`);
  }
} else if (command === 'standalone-packages') {
  if (!release) throw new Error('--tag is required');
  for (const artifact of release.artifacts.filter(item => item.kind === 'standalone')) {
    process.stdout.write([artifact.manifest_filename, artifact.filename].join('\t') + '\n');
  }
} else if (command === 'verify-assets') {
  if (!release || !revision) throw new Error('--tag and --revision are required');
  const directoryIndex = args.indexOf('--asset-dir');
  if (directoryIndex < 0 || !args[directoryIndex + 1]) throw new Error('--asset-dir is required');
  const directory = path.resolve(args[directoryIndex + 1]);
  const crypto = await import('node:crypto');
  let count = 0;
  for (const artifact of release.artifacts) {
    if (artifact.filename !== expectedFilename(artifact)) {
      throw new Error(`artifact filename differs from release tag: ${artifact.filename}`);
    }
    for (const asset of artifactAssets(release, artifact)) {
      const file = path.join(directory, asset.filename);
      const stat = fs.lstatSync(file);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size !== asset.size) throw new Error(`asset size/type mismatch: ${asset.filename}`);
      const digest = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
      if (digest !== asset.sha256) throw new Error(`asset digest mismatch: ${asset.filename}`);
      count += 1;
    }
    if (artifact.kind === 'standalone') {
      const packageManifest = JSON.parse(fs.readFileSync(path.join(directory, artifact.manifest_filename), 'utf8'));
      if (packageManifest?.schema !== 'hiroute.standalone-package/v1'
          || packageManifest.version !== release.version
          || packageManifest.revision !== revision
          || packageManifest.target !== artifact.target
          || packageManifest.distribution !== 'integration-candidate'
          || packageManifest.archive?.filename !== artifact.filename
          || packageManifest.archive?.sha256 !== artifact.sha256
          || packageManifest.archive?.size !== artifact.size) {
        throw new Error(`standalone package identity differs from release record: ${artifact.filename}`);
      }
    }
  }
  process.stdout.write(`${count} release asset(s) verified\n`);
} else if (command === 'verify-desktop-identity') {
  if (!release || !revision) throw new Error('--tag and --revision are required');
  const filenameIndex = args.indexOf('--filename');
  const identityIndex = args.indexOf('--identity');
  if (filenameIndex < 0 || !args[filenameIndex + 1]) throw new Error('--filename is required');
  if (identityIndex < 0 || !args[identityIndex + 1]) throw new Error('--identity is required');
  const artifact = release.artifacts.find(item => item.kind === 'desktop' && item.filename === args[filenameIndex + 1]);
  if (!artifact) throw new Error(`desktop artifact is not declared by ${tag}: ${args[filenameIndex + 1]}`);
  const identity = JSON.parse(fs.readFileSync(path.resolve(args[identityIndex + 1]), 'utf8'));
  verifyDesktopIdentity(release, artifact, identity, revision);
  process.stdout.write(`${artifact.filename} identity verified\n`);
} else {
  throw new Error(`unknown command: ${command}`);
}
