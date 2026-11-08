# HiRoute website

This is the static source for `https://hiroute.ai`. It uses Astro and emits only
files suitable for OSS static hosting. There is no website backend.

## Local development

Node.js 22 or later is required.

```sh
cd apps/website
npm ci --ignore-scripts
npm test
npm run dev
```

`npm run build` copies the repository's canonical custom extension API OAS and current
documentation illustrations into generated public directories, builds the site,
and checks required routes and local links. Product guides are authored once in
`content/guides`. Decision models, the custom extension API and Jev reference deployment render directly from
`decision-extensions`; do not copy their Markdown into this app. Generated
`public/api`, `public/decision-assets`, `public/install`, and `public/install.sh`
content is ignored by Git. The last two are generated from the canonical
standalone installer and the current stable release record; do not hand-edit them.

Session timeline images on the homepage and in the model-routing guide are frames from
real Desktop session recordings. Trust, report and configuration illustrations use actual
product components with constructed documentation data; see
[capture provenance](../../decision-extensions/assets/README.md) for their distinct sources.
They are illustrations, not provider acceptance or model benchmarks. Historical news and experiment
reports retain their original version and evidence; current setup instructions belong in the guides.

The guides separate model routing and Agent routing into independent walkthroughs.
Their bilingual dark-mode interaction screenshots live in `public/guide-assets`;
see [guide screenshot sources](docs/guide-assets.md) for production component
provenance, reproduction and caption conventions. Regenerate them with
`node apps/website/scripts/capture-guide-assets.mjs` from the repository root.

Each user guide includes an `aside.agent-guide` near its introduction, anchored at `#agent-cli`, with
Agent-facing instructions in one `text` code fence inside a native
`details.agent-guide-preview`. Include the canonical page URL, CLI reference,
built-in Skill setup links and concise UI-to-command pointers. Ask the Agent to
use the user's goal and installed help/schema rather than copying a fixed script.
The product's management and delegation Skills own the execution workflow;
Quickstart's `#management-skill` and the Agent-routing guide's `#delegation-skill`
show how to enable them. UI and CLI share the same service and underlying data.
`public/scripts/guide-agent-copy.js` adds a visible Copy for Agent button that
copies the complete message even with its preview closed. A failed clipboard
write opens and selects the text for manual copying; without JavaScript the
native preview remains readable. Keep both languages aligned. Detailed command
and recovery rules stay in the CLI guide and installed Skills.

English lives at `/en/` and Chinese at `/zh/`; `src/lib/locale.mjs` builds both
prefixes. The root and the Chinese URLs published without a prefix before English
became the default (`LEGACY_CHINESE_PATHS`) are redirect pages: an immediate meta
refresh to the destination with a matching canonical, which Google treats as a
permanent redirect. `public/scripts/language.js` runs first to keep the query and
fragment, and at the root opens `/zh/` for a saved or explicit (`?lang=zh`) Chinese
choice. Keep a former URL in that list while its Chinese page exists; new pages have
no unprefixed URL. Browser language never redirects: when it differs from the page,
a dismissible bar suggests the counterpart page, and dismissing it keeps the current
language. Search engines and shared links therefore receive the page they request.
All pages are static directory indexes; `/releases/*` is
never part of the website build and must remain a real object/404 boundary.

## Search metadata

Every public page has its own title and description: guides in
`src/lib/user-guides.mjs`, decision documents in `src/lib/decision-docs.mjs`,
articles in `news/index.json`, and other pages in their component. Keep them
specific to the page, factual for the shipped product and free of release-version
wording. Each page is its own canonical URL and declares the same `zh`/`en` pair,
with English as `x-default`; redirect pages have no metadata of their own and stay
out of the sitemap. `src/lib/seo.mjs` builds the JSON-LD: site, organization
and software on the homepage, `BlogPosting` for articles and breadcrumbs for docs
and articles. The sitemap repeats the language pair and sets `lastmod` to the newest
commit touching each page's own sources (`pageSources`), so it needs full Git
history; a shallow clone omits `lastmod`. `npm run build` rejects missing or
repeated titles and descriptions, inconsistent language alternates, invalid
JSON-LD, pages missing from the sitemap, malformed redirect pages and links to them.
The root page also carries the Baidu Search Resource Platform verification tag;
keep it while the site stays verified there.

## Articles and case studies

The repository-root `news/index.json` indexes bilingual articles in `news/`.
Both GitHub and the website use those same Markdown sources. Add an entry with a
unique slug, date, title, description, and file for each language; the article lists,
language links and newest-article homepage entry follow the index automatically.
For an older introduction with no recorded publication date, omit `date`; it
appears after dated articles and leaves the latest-article homepage entry intact.
An optional localized `shortTitle` keeps the homepage link concise; without one,
it uses the full article title. Public article URLs remain under `/news/`.
Article images live in `news/assets/` and are copied to ignored `public/news-assets/`
at build time. Evidence and reproduction commands belong in the public
`experiments/` directory. Keep these directories self-contained.

## Public release contract

`data/releases.json` is the single source for the download page and changelog.
Add a release only after its packages have completed integrity checks,
installation review, applicable component-notice review, and release approval.
Near-term releases may be self-signed; signing and notarization are not claimed
unless the exact artifact declares `developer-id`. The following template uses
`<release-version>`, `<publication-time>`, and `<revision-prefix>` placeholders;
replace them with the verified release facts before validating a record:

```json
{
  "version": "<release-version>",
  "channel": "stable",
  "published_at": "<publication-time>",
  "notes": { "zh": "版本说明。", "en": "Release notes." },
  "artifacts": [{
    "kind": "desktop",
    "platform": "macOS",
    "architecture": "arm64",
    "format": "dmg",
    "minimum_os": "15.0",
    "distribution": "self-signed",
    "filename": "HiRoute-<release-version>-<revision-prefix>-macos-arm64-trial.dmg",
    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "size": 123456789
  }, {
    "kind": "standalone",
    "platform": "Linux",
    "architecture": "x86_64",
    "target": "x86_64-unknown-linux-gnu",
    "format": "tar.gz",
    "distribution": "unsigned",
    "filename": "hiroute-<release-version>-<revision-prefix>-x86_64-unknown-linux-gnu.tar.gz",
    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "size": 98765432,
    "manifest_filename": "hiroute-<release-version>-<revision-prefix>-x86_64-unknown-linux-gnu.tar.gz.json",
    "manifest_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "manifest_size": 4096
  }]
}
```

The filename must be the exact output of `scripts/package-desktop.py`:
`self-signed` uses the `-trial.dmg` output and `developer-id` uses
`-developer-id.dmg`. Standalone filenames and companion manifests must be the
exact outputs of `scripts/package-standalone.py`; Linux packages are unsigned and
are described as checksum-verified, never code-signed. A development candidate is
not public merely because the schema accepts its distribution; it still needs the
reviews above. The website
derives the immutable URL rather than storing another configurable URL:

```text
https://hiroute.ai/releases/<version>/<packager-filename>
```

For a finalized release, list its version in `data/short-downloads.json`. The
website links then use `HiRoute-<version>.exe`, `HiRoute-<version>-macos-<arch>.dmg`,
and `HiRoute-<version>-linux-<arch>.tar.gz` with its `.json` companion under the
same version directory. The release workflow publishes both names from the
same verified bytes and checks their size and SHA-256 before deploying the
site. The Linux installer accepts the short archive name with the original
verified manifest. The GitHub assets and update feeds continue to use the
packager filenames, including the source revision.

The canonical manifest uses schema `hiroute.website.releases/v3`. Preparation emits
`releases-v3.json` for Windows Desktop and a `releases.json` V2 projection without
Windows artifacts for supported older macOS readers. Linux archives include the
Web UI assets as well as the CLI and service; this does not change their
`standalone` package kind.

The manifest intentionally has no
release-level Git revision. A release record cannot contain the SHA of the commit
that contains that same record without becoming self-referential. The packager
filename still includes the first 12 characters of its source revision.

Create tag `v<version>` on the exact source candidate, create a draft GitHub
Release, and attach every DMG, Linux archive and companion manifest, and the x64
Windows NSIS installer with its `.identity.json` sidecar when Windows is in scope.
Windows headless archives are internal payloads and must not be published. The
installer identity must bind the exact source/hash and native cases
from `scripts/windows-desktop-cases.json`, with the actual OS/build recorded.
Complete acceptance is the default; an explicit committed release decision can
defer named cases only for its exact version/source/installer, preserving their
`not_assessed` verdicts. Actual standard-user installation and console readiness
remain required for both selections. A deferral is never reported as a native pass.
Compilation alone does not establish native acceptance. After
reviewing those assets, add their exact filenames, sizes, and digests to the
manifest on `main`. Publish the GitHub Release only after that record lands.
The workflow freezes that `main` revision for the website, resolves the tag's
full commit independently, and verifies that every filename and package-internal
version/revision/architecture/distribution matches it. It also mounts and checks
DMGs and verifies standalone archive contents before publication. Developer ID
artifacts additionally pass the packager's stapling and Gatekeeper checks.

Release objects are create-once: an identical existing object is accepted, a
different object at the same key fails, and the workflow never overwrites it.
Wait for the final package selection before enabling short download names.
Replacing bytes at an already-published short URL requires a new version.
Only after all objects exist with matching size and SHA metadata does it deploy
the frozen current website. Hosted CI does not build or silently change an
artifact's distribution.

The generated `/install.sh` selects a stable Linux package from this same manifest
and delegates installation to the canonical `scripts/install-standalone.py`. With
no stable package for the host architecture it fails closed; it never selects a
preview or unlisted candidate and never starts the service automatically.

## Model feed

The independent `model-feed` workflow checks the complete metadata source and its
generated projections, then validates the publication with shared Rust admission
and Node structural checks. Metadata or announcement changes merged to `main`
trigger feed deployment without rebuilding the website. The validated payload
travels in its own artifact, outside the website `dist`. Its production job checks
the current HTTPS publication and uploads only `/model-feed/v2.json`, using the same
`website-production` environment and deployment lock as website/release uploads.
PR builds publish nothing. Unchanged and bootstrap-null payloads skip upload;
the first publication requires a recorded compatible client release. No feed
signing key is required. Static website uploads refuse a `model-feed` prefix.

The feed contains the complete model metadata; `featured.json` controls optional
recommendations separately. Clients have no bundled recommendations, so an absent
or empty selected feed list leaves Home/trial recommendations empty while model
metadata retains its installed fallback.

See the [feed maintainer guide](../../assets/model-feed/README.md) for regeneration,
sequence rules and release compatibility. OSS upload credentials are described below.

## OSS deployment

The `website` workflow builds on pull requests. Pushes to `main` and explicit
dispatches on `main` deploy through the `website-production` GitHub environment. Configure
the same secret names as the Higress website:

- `ACCESS_KEYID`
- `ACCESS_KEYSECRET`

The fixed storage configuration is:

- bucket: `hiroute-ai`
- endpoint: `oss-cn-hongkong.aliyuncs.com`
- region: `cn-hongkong`

Website deployment uses short caching for mutable pages and immutable caching for
hashed Astro assets. Release packages use immutable caching and SHA256 metadata.
Neither workflow deletes objects or synchronizes the whole bucket destructively.
When the manifest is nonempty, ordinary website deployment first checks that every
referenced release object exists with matching size and SHA metadata; broken links
therefore fail deployment without replacing the currently served site.

DNS, HTTPS, custom-domain routing, bucket read policy, and the rule that missing
`/releases/*` and `/model-feed/*` objects return a real 404 without redirects remain
infrastructure configuration outside this repository. A gateway may answer the
redirect pages with server-side 301 responses; they must use the same destinations. Verify those once `hiroute.ai` is connected to the bucket.
