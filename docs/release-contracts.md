# Release contract records and upgrade navigation

These are release facts, not an automatic promise to migrate every recorded release.
The first supported upgrade baseline is 0.2.0 once it is published and its snapshot is
merged. Historical 0.1.0 remains recorded but is outside the supported migration scope.

## Read the recorded release

The version is data, not a skill constant. From the repository root run:

```sh
python3 scripts/release-contracts.py check
python3 scripts/release-contracts.py current
```

PR CI also compares every historical snapshot byte-for-byte with the actual PR base; editing
or deleting one fails even if the index is recomputed. Locally use `check --base BASE_REF` for
the same check. The index itself is regenerated as new snapshots are appended.

The [index](../contracts/releases/index.v1.json) selects the highest recorded stable
semantic version. Each adjacent `<tag>.json` snapshot freezes the exact published commit,
repository/tag/channel, publication time, installer names/sizes/SHA-256, released SQL schema,
compatibility register and CLI contract-set descriptor. The register and descriptor retain
both their original source-byte digest and parsed content. Read other affected types, schemas
and producer behavior with `git show REVISION:path`; the current file is not the old contract.
The snapshot is evidence, not another current contract producer or a promise of migration support.

Fetch missing source objects/tags from the configured public remote without replacing an existing
tag. A missing/moved tag or index/hash mismatch must be resolved, not regenerated into agreement.
Never use a branch tip or a same-version development package as the released source.

Before compatibility work and release handoff, compare the index with the observed public stable
Release channel (use the repository in the snapshot). A newly published Release may have its
snapshot PR pending. Resolve that gap through the release workflow/PR before treating the recorded
baseline as current. Retain all older sources still declared supported; `latest_stable` is not an
end-of-support rule. The retired schema22 trial recovery catalog is no longer an upgrade entry;
published 0.1.0 records remain immutable historical facts, while 0.2.0 is the first supported baseline.

## Publication and retry

[Release workflow](../.github/workflows/release.yml) verifies the published assets and
package identities, then publishes the installers/site. Only after those jobs succeed does
`record-contracts` call [the PR publisher](../.github/scripts/release-contract-pr.sh).
It reads the verified manifest revision and exact released source, cross-checks published asset
sizes/digests, and proposes only `contracts/releases/` changes on `chore/release-contracts-<tag>`.

Normal review/checks and merge into `main` advance the repository's baseline. The workflow never
approves or merges its own PR. Existing snapshots are immutable; older reruns and previews retain
the highest stable baseline. A repeat reuses the same open PR; after merge it is a no-op. Failed
PR creation leaves a retryable branch. A closed PR, conflicting histories or unrelated edits on
that branch require review rather than force-pushing over them. With two pending release PRs,
merging one may cause an index conflict in the other; retain both snapshots, take the merged
main's index, and rerun `record` for the latter release with its verified inputs, then run `check`.

The job needs `contents: write` and `pull-requests: write`, plus the repository Actions setting
that allows GitHub Actions to create pull requests. GitHub exposes that setting together with
permission to approve; this workflow contains no approval action. PR checks must run through
the ordinary pull-request event and repository review policy. If GitHub marks the bot-created
workflow run as awaiting approval, an authorized reviewer must approve that run; a manual
`workflow_dispatch` run is not a substitute for required PR checks. See the
[GitHub event-trigger rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow).

If publication succeeds but recording fails, fix the reported cause and rerun the failed job.
This does not undo the published release. Initial backfill uses `gh release view TAG --repo REPO
--json tagName,isDraft,isPrerelease,publishedAt,assets` saved to a local file, followed by:

```sh
python3 scripts/release-contracts.py record --release-json /path/to/release.json \
  --manifest /path/to/verified-releases.json --revision FULL_SOURCE_SHA --repository OWNER/REPO
python3 scripts/release-contracts.py check
```

The manifest must be the one verified against the published package bytes and source identity.
Generate twice and confirm the second run has no diff. Commit the snapshot/index through a normal
PR. Do not edit historical snapshots or overwrite a collision to get a green check. Publication
records and contract-source checks do not establish real old-package upgrade/manual restoration;
supported-source acceptance remains a separate release obligation after the first 0.2.0 baseline.

## Find the current storage and upgrade boundaries

Inspect the current support register, dispatch and affected tests; this navigation does not fix
any particular product or SQL version as the target.

| Responsibility | Existing entry |
| --- | --- |
| Current contracts, retained readers and fixtures | [compatibility support register](../contracts/compatibility-support.v1.json) |
| First supported baseline | 0.2.0; the schema22 trial recovery catalog and package-selection entry have been retired |
| Startup, backup batches and SQL/payload dispatch | `crates/local-storage/src/lib.rs`, `crates/local-storage/src/migrations/mod.rs` and its dispatched modules, daemon startup |
| Startup format admission and current integrity | `crates/local-storage/src/migrations/startup_format.rs`, `current_storage.rs`; unsupported old stores are preserved and refused |
| Stable facts and execution projection | `crates/domain/src/routing/stored.rs`, `stored_capability.rs`, `capability_compiler.rs`; publication/Operation stored codecs |
| Operation, secret and native recovery | `crates/domain/src/operation/`, `crates/local-storage/src/secrets/`, `crates/local-storage/src/agents/` |
| Public CLI | Frozen CLI descriptor in the release snapshot; schemas and consumer/application-api code at its revision |
| Gateway semantics | Affected ingress/serializer/response code at the released revision and production listener tests |
| Current source fixtures and restart regressions | Registered fixtures, stable-codec tests and production publication/recovery scenarios |

Trace concrete format markers, complete-source backup rules and the three-store coordinator as
well as SQL dispatch. A future migration is not implemented merely because its number appears
in documentation. Users need not install intermediate binaries when the current startup actually
implements and validates the complete supported source-to-current chain.
