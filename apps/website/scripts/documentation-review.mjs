#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const repository = path.resolve(import.meta.dirname, '../../..');
const schema = 'hiroute.documentation-review/v1';
const tagPattern = /^v[0-9]+\.[0-9]+\.[0-9]+(?:[+-][0-9A-Za-z.-]+)?$/;
const revisionPattern = /^[0-9a-f]{40}$/;
const reviewDirectory = 'apps/website/docs/release-reviews';
const groups = {
  guides: ['README.md', 'README.zh-CN.md', 'apps/website/content/guides', 'apps/website/src',
    'apps/website/public/scripts', 'decision-extensions'],
  screenshots: ['apps/website/public/guide-assets', 'decision-extensions/assets', 'apps/website/docs/guide-assets.md'],
};
const digest = value => crypto.createHash('sha256').update(value).digest('hex');

function requireValue(condition, message) { if (!condition) throw new Error(message); }
function identity(tag, revision) {
  requireValue(tagPattern.test(tag ?? ''), 'use a product release tag such as v1.2.3');
  requireValue(revisionPattern.test(revision ?? ''), 'use the exact 40-character product revision');
}

function walk(root, relative) {
  const stat = fs.lstatSync(path.join(root, relative));
  requireValue(!stat.isSymbolicLink(), `documentation input must not be a symlink: ${relative}`);
  if (stat.isFile()) return [relative];
  requireValue(stat.isDirectory(), `unsupported documentation input: ${relative}`);
  return fs.readdirSync(path.join(root, relative)).filter(name => !name.startsWith('.')
    && !['node_modules', 'target', '__pycache__'].includes(name)).sort().flatMap(name => walk(root, `${relative}/${name}`));
}

export function documentationSnapshot(root = repository) {
  return Object.fromEntries(Object.entries(groups).map(([group, roots]) => {
    const files = roots.flatMap(relative => walk(root, relative)).filter(relative => {
      if (relative.split('/').at(-1).startsWith('.')) return false;
      if (group === 'screenshots') return /\.(?:png|webp|svg|jpg|jpeg|md)$/.test(relative);
      if (relative.startsWith('decision-extensions/')) {
        return !relative.startsWith('decision-extensions/assets/') && (relative.endsWith('.md')
          || relative.startsWith('decision-extensions/api/') && relative.endsWith('.json'));
      }
      return true;
    }).sort();
    requireValue(files.length > 0, `no ${group} inputs to review`);
    const entries = files.map(file => [file, digest(fs.readFileSync(path.join(root, file)))]);
    return [group, { content_sha256: digest(JSON.stringify(entries)), files }];
  }));
}

export function prepareDocumentationReview(root, tag, revision) {
  identity(tag, revision);
  const snapshot = documentationSnapshot(root);
  return { schema, tag, product_revision: revision, ...Object.fromEntries(Object.entries(snapshot).map(([group, value]) =>
    [group, { status: 'pending', summary: '', reviewed_files: [], content_sha256: value.content_sha256 }])) };
}

export function checkDocumentationReview(root, tag, revision) {
  identity(tag, revision);
  // Re-publication of an exact recorded release must not demand that today's
  // current-product guides be rolled back to a historical release's UI.
  const recorded = path.join(root, 'contracts/releases', `${tag}.json`);
  if (fs.existsSync(recorded)) {
    const snapshot = JSON.parse(fs.readFileSync(recorded, 'utf8'));
    requireValue(snapshot.schema === 'hiroute.release-contract-snapshot/v1' && snapshot.release?.tag === tag,
      `invalid recorded release: ${tag}`);
    if (snapshot.release.revision === revision) return { tag, revision, status: 'recorded-release' };
  }
  const file = path.join(root, reviewDirectory, `${tag}.json`);
  requireValue(fs.existsSync(file), `${tag}: missing documentation review; run documentation-review.mjs prepare after reviewing guides and screenshots`);
  const review = JSON.parse(fs.readFileSync(file, 'utf8'));
  requireValue(review.schema === schema && review.tag === tag && review.product_revision === revision,
    `${tag}: documentation review does not match the exact product revision`);
  const current = documentationSnapshot(root);
  for (const [group, snapshot] of Object.entries(current)) {
    const entry = review[group];
    requireValue(entry && ['updated', 'unchanged'].includes(entry.status), `${tag}: ${group} review is pending`);
    requireValue(typeof entry.summary === 'string' && entry.summary.trim().length > 0,
      `${tag}: explain the ${group} update or why the existing content remains accurate`);
    requireValue(Array.isArray(entry.reviewed_files) && entry.reviewed_files.length > 0
      && new Set(entry.reviewed_files).size === entry.reviewed_files.length
      && entry.reviewed_files.every(file => snapshot.files.includes(file)), `${tag}: name concrete reviewed ${group} files`);
    requireValue(entry.content_sha256 === snapshot.content_sha256,
      `${tag}: ${group} changed after review; inspect the changes and renew the review`);
  }
  return { tag, revision, status: 'reviewed', guides: review.guides.status, screenshots: review.screenshots.status };
}

export function checkAllDocumentationReviews(root = repository) {
  const directory = path.join(root, reviewDirectory);
  if (!fs.existsSync(directory)) return [];
  return fs.readdirSync(directory).filter(name => name.endsWith('.json')).sort().map(name => {
    const value = JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8'));
    return checkDocumentationReview(root, name.slice(0, -5), value.product_revision);
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [command, ...args] = process.argv.slice(2);
    const options = new Map();
    for (let index = 0; index < args.length; index += 2) {
      requireValue(['--tag', '--revision', '--repo'].includes(args[index]) && args[index + 1]
        && !options.has(args[index]), 'usage: documentation-review.mjs prepare|check|check-all [--tag TAG --revision SHA] [--repo PATH]');
      options.set(args[index], args[index + 1]);
    }
    const root = path.resolve(options.get('--repo') ?? repository);
    const tag = options.get('--tag'), revision = options.get('--revision');
    const actions = { prepare: () => prepareDocumentationReview(root, tag, revision),
      check: () => checkDocumentationReview(root, tag, revision), 'check-all': () => checkAllDocumentationReviews(root) };
    requireValue(Object.hasOwn(actions, command), 'choose prepare, check or check-all');
    console.log(JSON.stringify(actions[command](), null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
