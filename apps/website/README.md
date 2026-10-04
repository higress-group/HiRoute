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

`npm run build` copies the repository's canonical Decision API OAS and current
documentation illustrations into generated public directories, builds the site,
and checks required routes and local links. Product guides are authored once in
`content/guides`. The Decision API and Jev articles render directly from
`decision-extensions`; do not copy their Markdown into this app. Generated
`public/api`, `public/decision-assets`, `public/install`, and `public/install.sh`
content is ignored by Git. The last two are generated from the canonical
standalone installer and the current stable release record; do not hand-edit them.

Chinese lives at `/`; English lives at `/en/`. Language is an explicit link, not
an IP-based redirect. All pages are static directory indexes; `/releases/*` is
never part of the website build and must remain a real object/404 boundary.

## News and experiment articles

The repository-root `news/index.json` indexes bilingual articles in `news/`.
Both GitHub and the website use those same Markdown sources. Add an entry with a
unique slug, date, title, description, and file for each language; the news lists,
language links and newest-article homepage entry follow the index automatically.
Article images live in `news/assets/` and are copied to ignored `public/news-assets/`
at build time. Evidence and reproduction commands belong in the public
`experiments/` directory. Keep these directories self-contained.

## Public release contract

`data/releases.json` is the single source for the download page and changelog.
It is intentionally empty until a public package has completed integrity checks,
installation review, applicable component-notice review, and release approval.
Near-term releases may be self-signed; signing and notarization are not claimed
unless the exact artifact declares `developer-id`. A self-signed record looks like:

```json
{
  "version": "0.1.0",
  "channel": "stable",
  "published_at": "2026-09-21T00:00:00Z",
  "notes": { "zh": "版本说明。", "en": "Release notes." },
  "artifacts": [{
    "kind": "desktop",
    "platform": "macOS",
    "architecture": "arm64",
    "format": "dmg",
    "minimum_os": "15.0",
    "distribution": "self-signed",
    "filename": "HiRoute-0.1.0-aaaaaaaaaaaa-macos-arm64-trial.dmg",
    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "size": 123456789
  }, {
    "kind": "standalone",
    "platform": "Linux",
    "architecture": "x86_64",
    "target": "x86_64-unknown-linux-gnu",
    "format": "tar.gz",
    "distribution": "unsigned",
    "filename": "hiroute-0.1.0-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz",
    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "size": 98765432,
    "manifest_filename": "hiroute-0.1.0-aaaaaaaaaaaa-x86_64-unknown-linux-gnu.tar.gz.json",
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

The manifest uses schema `hiroute.website.releases/v2` and intentionally has no
release-level Git revision. A release record cannot contain the SHA of the commit
that contains that same record without becoming self-referential. The packager
filename still includes the first 12 characters of its source revision.

Create tag `v<version>` on the exact source candidate, create a draft GitHub
Release, and attach every DMG, standalone archive, and companion manifest. After
reviewing those assets, add their exact filenames, sizes, and digests to the
manifest on `main`. Publish the GitHub Release only after that record lands.
The workflow freezes that `main` revision for the website, resolves the tag's
full commit independently, and verifies that every filename and package-internal
version/revision/architecture/distribution matches it. It also mounts and checks
DMGs and verifies standalone archive contents before publication. Developer ID
artifacts additionally pass the packager's stapling and Gatekeeper checks.

Release objects are create-once: an identical existing object is accepted, a
different object at the same key fails, and the workflow never overwrites it.
Only after all objects exist with matching size and SHA metadata does it deploy
the frozen current website. Hosted CI does not build or silently change an
artifact's distribution.

The generated `/install.sh` selects a stable Linux package from this same manifest
and delegates installation to the canonical `scripts/install-standalone.py`. With
no stable package for the host architecture it fails closed; it never selects a
preview or unlisted candidate and never starts the service automatically.

## OSS deployment

The `website` workflow builds on pull requests. Pushes to `main` and explicit
dispatches deploy through the `website-production` GitHub environment. Configure
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
`/releases/*` objects return a real 404 remain infrastructure configuration outside
this repository. Verify those once `hiroute.ai` is connected to the bucket.
