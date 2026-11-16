import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync } from 'node:fs';
import { validateReleaseManifest } from '../../apps/website/src/lib/releases.mjs';

const catalog = 'apps/website/data/releases.json';
const websiteInputs = [
  'apps/website', 'news', 'experiments', 'decision-extensions',
  'apps/desktop/src-tauri/icons/icon.png', 'scripts/install-standalone.py',
  'scripts/windows-desktop-cases.json', '.github/scripts',
  '.github/workflows/website.yml', '.github/workflows/release.yml',
];

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8' });
}

function catalogTags(contents) {
  const manifest = validateReleaseManifest(JSON.parse(contents));
  return manifest.releases.map(({ version }) => `v${version}`);
}

function publicationDeferral(before) {
  const recorded = JSON.parse(readFileSync('contracts/releases/index.v1.json', 'utf8'));
  const recordedTags = new Set(recorded.releases.map(({ tag }) => tag));
  const unrecorded = catalogTags(readFileSync(catalog, 'utf8')).filter(tag => !recordedTags.has(tag));
  if (unrecorded.length === 0) return;

  // A push adding a version belongs to publish-release, even if the GitHub
  // Release is already public by the time this build completes.
  if (before && before !== '0'.repeat(40)) {
    if (!/^[0-9a-f]{40}$/.test(before)) {
      throw new Error('Expected the full pre-push commit SHA');
    }
    const previous = new Set(catalogTags(git('show', `${before}:${catalog}`)));
    const added = unrecorded.filter(tag => !previous.has(tag));
    if (added.length) {
      return `The new catalog will deploy through publish-release for ${added.join(', ')}.`;
    }
  }

  // Follow-up pushes can inherit a pending release. A recorded tag or a
  // published GitHub Release still requires the normal OSS integrity checks.
  const published = execFileSync('gh', [
    'api', '--paginate', `repos/${process.env.GITHUB_REPOSITORY}/releases?per_page=100`,
    '--jq', '.[] | select(.draft == false and .published_at != null) | .tag_name',
  ], { encoding: 'utf8', timeout: 30_000 });
  const publishedTags = new Set(published.trim().split('\n'));
  const pending = unrecorded.filter(tag => !publishedTags.has(tag));
  if (pending.length) {
    return `Waiting for GitHub publication of ${pending.join(', ')}.`;
  }
}

function staleBuildDeferral() {
  // Check inside the shared production lock. Builds can enter the queue out
  // of push order, and a newer main commit can contain only product changes.
  git('fetch', '--no-tags', '--depth=1', 'origin', 'refs/heads/main');
  if (git('diff', '--name-only', 'HEAD', 'FETCH_HEAD', '--', ...websiteInputs).trim()) {
    return 'Skipping this build because website inputs have changed on main.';
  }
}

const [command, before] = process.argv.slice(2);
let deferral;
if (command === 'plan') {
  deferral = publicationDeferral(before);
} else if (command === 'current') {
  deferral = staleBuildDeferral();
} else {
  throw new Error('Usage: website-deployment.mjs <plan [BEFORE_SHA]|current>');
}
if (deferral) console.log(deferral);
appendFileSync(process.env.GITHUB_OUTPUT, `deploy=${!deferral}\n`);
