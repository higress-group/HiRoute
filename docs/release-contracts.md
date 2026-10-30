# Release contract records and upgrade navigation

These are release facts, not an automatic promise to migrate every recorded release.
The first supported upgrade baseline is 0.2.0 once it is published and its snapshot is
merged. Historical 0.1.0 remains recorded but is outside the supported migration scope.

## Upgrade and manual recovery

Supported upgrade readers accept the released sources declared by the support policy.
This does not promise that an older binary can read data written by a newer release.
Do not downgrade an initialized data directory in place. Manual recovery requires a
complete pre-upgrade backup and the corresponding binary verified to read it; restore
the whole store group, not selected tables or files.

Observation format 5 changes the physical protocol carriers. On a supported
format 1–4 upgrade, startup preserves the original three-store group, keys,
auxiliary state and matching old package, then seals the stopped Observation
DB/WAL/SHM/object/staging directory with a same-filesystem rename. It starts a
fresh format-5 store with current configuration, credentials and routing. Old
observations are not converted or merged into current queries; history and
statistics begin at cutover. Current-format stores and their capacity policy are
preserved on ordinary restart.

Desktop reuses the verified previous package from its installation handoff. A
headless operator must retain the old package and provide its absolute directory
through `HIROUTE_RECOVERY_PACKAGE` on the first sealing startup. Missing keys or
recovery material stop startup without clearing data. Interrupted publication
resumes the original batch; missing or replaced archive material is not permission
to initialize an empty store. Unfinished older conversion generations remain
preserved for whole-group recovery.

Installers retain the matching old program/resources through a protected handoff bound
to the storage root and target daemon image. Linux managed services and Windows Tasks
consume that record without relying on the shell's environment. New candidate preflight
checks observable prerequisites before replacement; startup repeats admission under its
writer barrier. The actual updater shipped by the supported source must be tested. In
particular, v0.3.1 runs its own helper: a new helper does not retrofit its precommit checks.
A format-changing release requires a verified source/transition route before publication;
documentation or a rebuilt same-version updater is not that evidence.

On startup refusal, the storage-owned cause/action is available through the native failure
page and `hiroute service recovery --output json`. Current production recovery does
not offer program-only original-version reinstallation. A changed/mixed/unknown group
cannot authorize an old-service launch, including during Windows installer rollback.
Recognized batches may resume with the current version; complete restoration uses the
correct verified batch, retaining newer data and program separately. Missing essential
material requires preservation and diagnosis, not blind retries or empty initialization.

Startup does not enumerate, hash, copy or normalize historical Observation
contents. The source manifest verifies copied recovery material and binds the
sealed directory's native identity; it does not claim eager full-content integrity
verification. This local recovery point still occupies disk and is not an
independent disaster-recovery copy. Protected sources/packages are excluded from
the active Observation threshold and are never deleted by its capacity policy.

For manual recovery, stop the daemon, preserve later new-format data separately,
check the old source offline, and restore its complete original storage group
and auxiliary state at the recorded original locations with the matching old
package, following the generated recovery guide. Never mix old business stores
with newer Observation data or restore only `activity.db`. An independent copy
and full-content check can be made offline, outside service startup.

Before starting the old program, also move the adjacent
`<storage-directory-name>.upgrade-backups` directory to a distinct unused
name on the same filesystem and preserve it completely. Older programs inspect its active migration
metadata independently of the restored databases and can reject a newer batch.
The selected source and its guide may move with this directory; continue using
that guide's new containing directory. Do not delete or rewrite the preserved
batches. This step also applies to retained guides that do not yet mention it.

Branch-route publications may contain `source_binding_id` in the persisted active,
prepared or last-known-good Gateway aggregate. Version 0.3.1 and earlier strict readers
reject this field, even though the enclosing contract remains
`hiroute.gateway-publication/v3`. After such a publication, using an old binary against
the updated directory is unsupported. The current reader authenticates and rebuilds
older aggregates from their sealed Plans; the reverse path is not supported. Keep the
pre-upgrade backup if reverting to an earlier release may be necessary.

## Read the recorded release

The version is data, not a skill constant. From the repository root run:

```sh
python3 scripts/release-contracts.py check
python3 scripts/release-contracts.py current
```

PR CI also compares every historical snapshot byte-for-byte with the actual PR base; editing
or deleting its evidence fails even if the index is recomputed. An explicitly superseded
unpromoted snapshot is retained byte-for-byte in `archive/<tag>.<full-revision>.json`.
Locally use `check --base BASE_REF` for
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
Archived snapshots verify their original commit and contract bytes without requiring the
superseded tag to continue pointing at that commit; active snapshots still require the exact tag.

Before compatibility work and release handoff, compare the index with the observed public stable
Release channel (use the repository in the snapshot). A newly published Release may have its
snapshot PR pending. Resolve that gap through the release workflow/PR before treating the recorded
baseline as current. Retain all older sources still declared supported; `latest_stable` is not an
end-of-support rule. The retired schema22 trial recovery catalog is no longer an upgrade entry;
published 0.1.0 records remain immutable historical facts, while 0.2.0 is the first supported baseline.

## Publication and retry

Prepare the macOS ARM64/Intel and Linux x86_64/ARM64 assets with the
[release candidate builder](release-builds.md). It supplies measured website
artifact rows; this workflow consumes the reviewed catalog and published bytes.

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

### Explicit replacement before promotion

An owner may authorize replacing a same-version release before public promotion. This is an
exceptional publication decision, never inferred from the version number or a moved tag. Build
and verify the new exact source and package identities first; retain the old source and artifact
identities. Then manually dispatch `publish-release` with the tag and
`supersede_unpromoted_revision` set to the exact previous 40-character source SHA. The ordinary
release event and default dispatch cannot replace an existing record.

After package/site publication, recording archives the previous snapshot without changing any
byte and updates only the active same-tag record and index. The repository and channel must
remain unchanged and the new source revision must differ. The replacement PR uses
`chore/release-contracts-<tag>-<new-revision-prefix>` so a previously merged snapshot PR cannot
block the replacement. Review and merge it normally. Local recovery uses
`record --supersede-unpromoted-revision OLD_FULL_SHA` with the same verified publication inputs;
repeating it is idempotent, including recovery after the active record was written but its index
was not. `check --base BASE_REF` protects both prior archives and the newly archived old record.

Archived trial records do not introduce another supported migration baseline. For the authorized
0.2.0 replacement before promotion, the active verified 0.2.0 record establishes that baseline;
the earlier package identities remain historical evidence.

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
