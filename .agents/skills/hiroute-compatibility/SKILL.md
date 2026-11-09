---
name: hiroute-compatibility
description: Maintain HiRoute compatibility when changing persisted data, user configuration, recovery, published CLI/Gateway behavior or OS filesystem/process boundaries, and at release handoff. Resolve supported releases and platform contracts with regression evidence. Not for unrelated UI, docs, tooling or metadata edits.
---

# HiRoute compatibility maintenance

Keep supported released data and callers working through the current production boundaries.
Apply within the authorized task and ownership; this skill adds no approval, delegation or
publication step. It does not require a migration for every feature or internal refactor.

For filesystem, permissions, process launch, background lifecycle or packaging changes
that affect Windows and Unix, read [platform compatibility](references/platforms.md).
It owns platform-specific decision guidance; the existing validation skill owns execution.

## Resolve the supported source

Read [release records and support policy](../../../docs/release-contracts.md), then the affected
durable spec or Design. Use the existing tools from the repository root:

```sh
python3 scripts/release-contracts.py current
python3 scripts/release-contracts.py check --base BASE_REF
```

Replace `BASE_REF` with the task's actual comparison revision. At task start, compare the
recorded stable release with the public stable Release channel named in the snapshot; reuse
that observation within the task and refresh at release handoff. A pending snapshot is a
recording gap: resolve it for the target branch before declaring its baseline current.
Missing source, a moved tag or a hash mismatch must be investigated, not regenerated away;
continue independent work while the affected conclusion remains unresolved.

- Derive versions and exact source commits from the records, and supported sources from the
  support policy. `latest_stable` does not retire earlier supported releases. Recorded history,
  a recovery fixture or a parser accepting a format does not itself promise release support.
- Read affected types, schemas and producer behavior at the recorded revision with
  `git show REVISION:path`. The CLI snapshot is a descriptor, and the SQL number is only one
  format marker; neither substitutes for the released serializers, defaults or behavior.
  A branch tip or same-version development binary is not the released source.
- Trace the current reader, conversion/startup entry and affected tests using the release
  navigation and [compatibility register](../../../contracts/compatibility-support.v1.json).
  Follow stable payloads, configuration/native files, secrets and recovery references as
  applicable; equal SQL versions do not guarantee equal payloads.

Record the affected boundary, supported source revision, intended behavior, conversion entry
and required checks in the existing Design or task notes. For an unchanged boundary, give a
brief reason. Do not create a separate registry or receipt for every change.

## Fit the change to the existing boundary

| Change | Required treatment |
| --- | --- |
| Internal implementation or rebuildable cache; stable inputs retain their meaning | Change the current projection/compiler/cache handling and test that meaning. No storage migration solely for an internal Rust type change. |
| Compatible extension | Prove old bytes/calls retain their meaning, including strict readers, defaults, canonical digests and authorization. Added fields or enum values are not automatically compatible. |
| Incompatible persisted/configuration format | Version the affected format and extend its existing reader/conversion. Preserve supported old inputs and any intermediate format with retained real data; disposable unreleased experiment formats need no invented support promise. |
| Published CLI/Gateway behavior | Adapt or version at that boundary and test old calls, outputs/errors and semantics. Package-private component handshakes may still require matching versions. |

Keep one current producer and business model; historical readers normalize at their boundary.
When storage conversion changes, follow the actual three-store coordinator, format admission,
SQL dispatch and payload conversion, complete-source backup and completed/incomplete batch
handling. Preserve identities, reference closure, digest/ciphertext bindings, native ownership
and unfinished-operation recovery; do not replay completed effects or clear data to make it
readable. Keep recoverable intermediate states admissible until transaction recovery completes,
then enforce cross-store business integrity before opening service. Increasing a schema number
or adding an SQL file alone does not implement the path.

Extend only the boundaries the change affects; do not recreate retired recovery catalogs or
prewrite a future migration. Preserve unsupported/corrupt input without silently initializing
over it. Existing manual recovery pairs a complete old backup with its matching binary;
upgrade support does not promise old binaries can read newly written data.

Register added or changed legacy readers, adapters, frozen fixtures and catalog fallbacks with owner, reason and
removal condition in the existing compatibility register. Follow the support policy for any
retirement; a newer release or absence of old records on the developer's machine is insufficient.

## Validate the affected path

Use [HiRoute validation](../hiroute-validation/SKILL.md) and the existing test planner/runner. Select checks by the changed boundary;
this skill does not add a full workspace or physical-machine run to every task.

- Freeze affected old bytes/calls from the exact released producer, or reuse fixtures with
  that provenance. Never regenerate the old input or authentication proof with current code.
  Verify business facts, identity, authority and references, not just successful parsing.
- Cover current writes and reopen/idempotency when persistence changes. Conversion/recovery
  changes also need affected interruption, corruption, backup and restore assertions through
  the existing production startup/recovery entry. Include retained supported intermediate
  sources when their path changes; fixture-only success is not release-to-release acceptance.
- Deliver the compatibility change and its regressions together. Run contract convergence
  after contract changes, and use the validation workflow for affected generators. Bind
  results to the actual candidate; a descriptor/hash audit does not test semantic compatibility.

At release handoff, assess the accumulated diff against supported released sources and reuse
applicable revision-bound evidence. Validate affected old-to-new production paths with exact
released inputs/packages and the final candidate within the authorized validation scope.
Record validation scope and remaining evidence gaps in the release Issue or PR.
Keep migration tests scoped to affected boundaries. After publication, follow the existing
snapshot workflow and retain historical release records.

Follow the [public release notes guidance](../../../docs/release-builds.md#write-public-release-notes)
when publishing or editing a release.
