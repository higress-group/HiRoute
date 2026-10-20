# Model feed over HTTPS

The service retrieves `https://hiroute.ai/model-feed/v1.json`. This is a validated,
versioned projection of the existing model metadata source, with editorial
announcements. It does not introduce a second editable model catalog.

## Maintained inputs and generation

| File | Owner and meaning |
| --- | --- |
| `featured.json` | Editorial array of `new_model` and `model_update` announcements; plain bilingual text and links restricted to `https://hiroute.ai/` |
| `publication.json` | Publication `sequence` and UTC `issued_at`; increase the sequence whenever payload bytes change |
| `policy.json` | `minimum_feed_client`, the first feed-capable release |
| `payload.json` | Generated payload; never hand-edit model facts here |

The Desktop client also bundles the maintained `featured.json` editorial input.
When no feed announcements are available, Home and quick start use only these
resolvable announcements, rather than labeling the first directory entries as new.
Bundled announcements keep the model's built-in origin and never claim website retrieval. An update headline may name an announced release while its note
distinguishes that announcement from the version served by a connection. The
underlying model identity, connection checks, prices and published routing remain
owned by the ordinary catalog and saved connection. An unresolved announcement
is hidden in both sources.

Change model facts in
[`metadata-catalog.json`](../model-data/current/inputs/metadata-catalog.json), then
follow the [metadata maintainer workflow](../../.agents/skills/model-metadata-maintainer/SKILL.md).
Regenerate the current model data and release bundle before running:

```sh
cargo run -p hiroute-release-facts -- feed build
cargo run -p hiroute-release-facts -- feed build --check
node apps/website/scripts/model-feed.mjs check
```

The Rust producer uses the client's admission code and exact supported release
bundles from repository history. It computes the minimum admitting client version,
carries inclusion dates forward from the committed payload, and omits models waiting
for a compatible release. A model's minimum version requires its executable capability
and offer together. CI passes `--base <revision>` to reject changed publication bytes
without a higher sequence and changed first-inclusion dates for retained records.
The payload's `source_revision` is the Git tree ID of the committed
`assets/release-facts/current/bundle` directory. It survives squash merges and
reproduces a file with `git show <tree-id>:model-data.json`; it is provenance,
not each website deployment's commit SHA.
One sequence identifies one immutable payload digest and source revision. Keep the
referenced bundle tree reachable from main history. Node structural checks complement, and never replace, Rust admission validation.

Before a feed-capable release is recorded, `payload.json` contains `null` followed
by a newline. Both build checks report bootstrap; deployment skips network access
and feed publication and still uploads the ordinary website. Bootstrap is not a
production feed publication.

## Production publication and trust

The fixed HTTPS origin and the production website pipeline are the content trust
boundary. Clients validate TLS certificates, refuse redirects and production URL
overrides, and validate the direct `hiroute.model-feed/v1` JSON payload. There is
no signature envelope, compiled feed public key, signing secret or key rotation.
A digest binds exact publication bytes for cache integrity and sequence identity;
it is not independent publisher authentication. Feed records cannot introduce
connection endpoints, authentication or executable adapters. Embedded facts and
manual price overrides retain their existing precedence.

The [website workflow](../../.github/workflows/website.yml) runs the shared Rust
check and Node check on the same source revision before uploading the validated
payload as a separate artifact. PR jobs never publish. Only `main` production
pushes or dispatches enter the production environment. The deploy job downloads
that workflow revision's payload and runs:

```sh
node apps/website/scripts/model-feed.mjs publish --payload <validated-payload-path>
```

The publisher requests the fixed feed URL without redirects or cached-response
reuse. Only a real 404 permits first publication. Existing publications must pass
bounded JSON validation before their sequence or digest is used. Other HTTP
responses, network errors or invalid content fail deployment before upload.

An identical payload digest is a no-op: no feed file is written, and the existing
OSS object is preserved by the non-deleting upload. Different content requires a
strictly greater sequence. The script enforces the 2 MiB limit and atomically
stages the exact validated payload at `apps/website/dist/model-feed/v1.json`.
It refuses a website artifact that already contains a feed. Retrieval or validation
failure leaves the existing served website and feed intact. The job summary records
status, sequence, payload SHA-256 and source revision.

Keep the endpoint stable for supported clients. An incompatible future contract
needs a new versioned endpoint and compatibility-register entry. The direct JSON
contract is introduced before any supported feed-capable release; the abandoned
signed-envelope development format is not a supported legacy format.

Ensure infrastructure serves missing `/model-feed/*` objects as real 404s without
an HTML fallback or redirect. The existing short-cache OSS upload remains in use.
After the first feed-capable release, verify a real publication through both a
standalone Linux service and macOS Desktop. Controlled fixtures do not establish
live hiroute.ai retrieval. Keep sequence/checkpoint, private atomic cache, restart
validation, fallback and frozen-published-route lifecycle regressions in place.
