# Complete official model data over HTTPS

The daemon retrieves `https://hiroute.ai/model-feed/v2.json`. One complete public
body supplies the installed fallback and website publication. Maintain model facts
in the existing [model-data inputs](../model-data/current/inputs), not a second
website catalog. Public metadata includes records without an executable binding.

## Inputs and generation

| Input | Meaning |
| --- | --- |
| `assets/model-data/current/inputs/official-data-revision.json` | Monotonic semantic data revision; changed semantic content requires a higher revision |
| `assets/model-data/current/inputs/metadata-catalog.json` and existing runtime/rating inputs | Complete model, provider, product, binding, capability, documented price and provenance facts |
| `assets/release-facts/current/bundle/official-model-data.json` | Generated shared data revision, semantic digest and complete body |
| `featured.json` | Optional bilingual editorial announcements; links stay under `https://hiroute.ai/` |
| `publication.json` | Monotonic publication sequence and UTC issue time; changed exact bytes require a higher sequence |
| `policy.json` | Earliest intended reader; a version string does not prove a compatible reader shipped |
| `payload.json` | Generated publication, or `null` while no compatible supported client is recorded |

Follow the [metadata workflow](../../.agents/skills/model-metadata-maintainer/SKILL.md),
regenerate model data and the release bundle, then run the producer and check twice:

```sh
python3 assets/release-facts/current/prepare-bundle.py
python3 assets/release-facts/current/prepare-bundle.py --check
cargo run -p hiroute-release-facts -- feed build
cargo run -p hiroute-release-facts -- feed build --check
node apps/website/scripts/model-feed.mjs check
```

For a semantic change, increase the data revision before regeneration and commit
the reviewed inputs and generated bundle before building the feed. Then update the
publication sequence/time when exact publication bytes change, build/check the feed,
and commit its generated payload. This preserves the bundle-tree provenance through
a squash merge. Supply the PR/push comparison revision with `feed build --check
--base BASE`; HEAD also protects uncommitted regeneration.

The Rust producer validates the complete candidate even during bootstrap. Before
release gating it rejects an embedded revision rollback or different semantic content
at the same revision against HEAD and the supplied baseline, including when
`payload.json` remains `null`. It checks recorded supported source readers before
enabling publication. Unsupported execution
requirements remain descriptive; they do not remove otherwise valid metadata. The
publication's `source_revision` names the committed release-bundle tree, which is
stable through a squash merge. Keep that tree reachable in main history.

Semantic data identity and exact publication identity are separate. Reformatting or
editing announcements does not change authoring/price dependencies. A corrected or
withdrawn data fact requires a new data revision. Whole-snapshot selection never
fills missing fields from an older snapshot. Evidence dates, editorial inclusion
dates, retrieval times and publication times keep their distinct meanings.

## Contract and authority

The direct `hiroute.model-feed/v2` JSON carries sequence, issue time, source tree,
data revision/digest, complete data and optional featured entries. Limits are 16 MiB
of decoded bytes, depth 64, UTF-8 strings of 16 KiB, keys of 256 bytes and records
of 256 KiB, plus finite collection bounds. Duplicate keys, including escaped
spellings, are rejected. Integer literals retain signed/unsigned 64-bit precision;
decimal rates and descriptive decimal quantities use strings. Rust and Node share
[canonical identity goldens](../../contracts/model-feed/canonical-data.v1.json).

HTTPS certificate validation and the production website pipeline establish trust.
There is no signing key or signature envelope. Production refuses redirects and
caller-supplied URLs. Requests carry no account, credential, session or usage data.

The website cannot install registry entries, authentication, destinations, headers,
adapters or free entitlement. Executable qualification uses the installed registry
and actual source/account authority. Exact product/protocol associations govern
capability overrides and documented prices. Ambiguity and unsupported conditions
remain unknown. Prices preserve input, output, cache-read and cache-write separately;
display-only cost hints are never rates.

User-preserved values take priority. A saved field following an exact official
locator resolves the selected compatible data; old ambiguous/manual evidence is
preserved. Source-owned preference changes share source CAS, activation and recovery.
Published Plans retain sealed execution facts, and settled costs retain frozen quotes.
Each new logical request captures the current price handle.

## Cache and product reads

The daemon selects the highest compatible semantic revision of its verified cache
and embedded data. Independent v2 publication and semantic acceptance checkpoints
survive cache removal/corruption. The bounded retained-v1 settings converter preserves
original bytes and user choices without importing v1 delta payloads or sequence floors.

Network retrieval, parsing and qualification do not hold the Control writer. Matching
prices are prepared before activation. The final feed-to-Control writer barrier checks
all captured dependencies before committing acceptance and installing the pair.
Announcement-only updates do not rebuild prices or invalidate ordinary previews.

CLI `models metadata list/get` and all UI hosts read the same selected snapshot.
List pages default to 50, maximum 100, within a 1 MiB response budget. Cursors bind
snapshot/filter/order; an obsolete cursor requires a fresh first page. Removing a
cache requires a complete-impact digest and a bounded paginated preview of capability,
reasoning, price and new-use changes, including already embedded models. Published
revisions and user-preserved facts remain intact.

## Production publication

The independent [`model-feed` workflow](../../.github/workflows/model-feed.yml)
checks the complete metadata source, model-data and release-bundle projections,
then validates the publication with Rust client admission and Node on the same
revision. Model metadata, editorial announcements and recorded release changes
trigger it on PRs and pushes to `main`; it can also be dispatched on `main`.
After the workflow reaches the default branch, run it independently from
Actions → `model-feed` → **Run workflow**, selecting `main`, or with:

```sh
gh workflow run model-feed.yml --repo higress-group/HiRoute --ref main
```

Dispatching another branch validates its artifacts but does not publish. A manual
run retains the same bootstrap, compatibility and publication checks as a push.
Reviewed changes must include regenerated artifacts and the required semantic
revision/publication sequence increases. CI checks those bytes rather than
editing the catalog or committing generated changes.

Only its production deployment may upload the exact validated artifact to
`model-feed/v2.json`. It uses `website-production` and the existing OSS credentials,
serializing uploads with website/release deployment through `hiroute-ai-production`.
All three publishers queue pending uploads instead of canceling one another.
It needs no static website build, experiment downloads or release-package checks.
Ordinary website uploads refuse a `model-feed` prefix. PR builds publish nothing.
Bootstrap `null` skips network access, feed staging and OSS CLI installation while
normal website deployment continues. Release a
compatible v2 client and record its exact source before enabling the first publication.
No supported recorded release currently establishes a served v1 compatibility endpoint.

The feed carries the complete metadata catalog, capabilities, documented prices,
reference scores and connection hints. `featured.json` independently selects and
orders the optional Home/trial recommendations; metadata membership alone never
recommends a model. Clients show only entries in the selected validated feed,
including its retained cache, whose model references resolve. An absent or empty
list shows no recommendations, with no bundled announcement fallback. Clearing
recommendations changes publication bytes and requires a higher publication
sequence, but does not require a data revision increase by itself.

The publisher retrieves the fixed v2 URL without redirects. A genuine 404 permits
bootstrap; retrieval failure, invalid served content, publication rollback, semantic
rollback or equal-revision conflicting content stops replacement. Identical exact
bytes are a no-op. Otherwise, stage only the validated artifact at
`apps/website/dist/model-feed/v2.json`; non-deleting upload preserves the previous
object on failure. Record publication sequence/digest/source and data revision/digest.
Missing `/model-feed/*` objects must return actual 404s without an HTML fallback.
