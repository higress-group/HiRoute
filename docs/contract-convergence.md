# Contract convergence and compatibility

Every production boundary has one current write or ingestion contract. Older bytes may remain
only for an explicit recovery reader, a separately named frozen fixture, or a rejection test. A
legacy decoder authenticates the original bytes before conversion and never becomes a second live
producer.

Routing authoring accepts only `hiroute.plan-content-change/v2`; the former V1 create/update DTO
has no public codec or planner and remains solely as an exact rejection fixture. Publication
recovery accepts the two historical Product shapes actually written to disk: V2 with compiler/Plan
V1, and master `501453a2`'s compiler-V1 aggregates containing V1 and/or V2 Plans. The complete original
record is validated before `into_current` reseals compiler/Plan V2. Each nested Plan must validate its own schema/compiler/digest; unsupported aggregate
compilers and tampered nested Plans fail closed. Every constructor and subsequent publication write emits
V3/compiler V2/Plan V2.

## Machine audit

[`contracts/compatibility-support.v1.json`](../contracts/compatibility-support.v1.json) is the
machine-readable support register. Each exception names its owner, reason, measurable removal
condition, and exact paths. It also contains the sorted inventory of exact internal contract tokens
used by production Rust sources under `apps/*/src`, `crates/*/src`, and `tools/release-facts/src`.
Test-only Rust items and fixture trees are excluded from that inventory, while current and legacy
families in tests remain governed by the subject path rules.

`python3 scripts/check-contract-convergence.py` rejects an undeclared production token, a stale
inventory entry, a multi-version production family not classified by one subject, or a registered
legacy token outside its allowed paths. This catches both producer and consumer drift without
treating arbitrary prose or third-party fixtures as production contracts. It also pins database
schema V23 from the single production declaration
`LATEST_SCHEMA_VERSION` in `crates/local-storage/src/migrations/mod.rs`, and retains the
unversioned `control.sock` endpoint. The schema gate accepts exactly one plain, unattributed
top-level constant declaration in that source. Conditional declarations (including nested
`cfg`/`cfg_attr` expressions), module/macro bodies, comments and string literals cannot satisfy
it. Duplicate declarations and malformed lexical input fail. Inner file attributes require an
explicit gate review because they can condition the entire module; the checker does not evaluate
Rust configuration expressions. Matching literals in fixtures or other files cannot satisfy the
gate. This narrow lexical check is not a replacement for Rust compilation.

The local-storage owner owns both the schema and startup support policy. The current startup
path accepts a fresh store or the current V23 format; existing schema 22 and other unsupported
formats fail closed, as specified in [storage upgrades](upgrade-storage-design.md) and implemented
in `crates/local-storage/src/migrations/startup_format.rs`. The version pin is an explicit review
gate, not a claim that this Python script executes migration or recovery semantics. A schema
change must review the production declaration, startup policy and its Rust contract tests, then
update `EXPECTED_DATABASE_SCHEMA_VERSION` in the checker. Do not relax the gate by adding a
fixture literal or a second live producer. `python3 scripts/test-contract-convergence.py` checks
the gate itself, including changed production versions masked by historical fixtures.

Desktop bridge owns `hiroute.web-confirmation/v1`, the local Tauri event between its native backend
and bundled WebView confirmation host. V1 is the only emitted and accepted version; there is no
legacy reader. Remove the registry subject together with the backend producer and WebView consumer
if that local presentation boundary is retired.

## Persisted-data disposition

| Boundary | Preserved data | Conversion or current write | Removal evidence |
| --- | --- | --- | --- |
| Routing publications and compiled Plans | Original authenticated bytes/digest for the recovery decision; Plan heads, grants, lifecycle, aliases, and executable arrays | Reseal the whole aggregate as V3/compiler V2/Plan V2 before live use or the next write | Zero legacy tuples across active, prepared, last-known-good, journal, and supported-backup inventories |
| Publication operation markers | Operation/effect identity, workspace, revision, before/after digests, decision/no-op state, and the exact publication record reconstructed from the owning operation and intent | Normalize V1 to marker V2 at the common read/reconciliation boundary and persist V2 at the next checkpoint | Zero V1 markers in active/retained journals; every supported backup proves exact recovery or is explicitly retired |
| Secret entries and effect journals | Authenticated plaintext plus credential identity, generation, policy fields, and before/staged recovery slots | Atomically rewrap fully identified V1 AAD rows as V2 in V17; incomplete legacy-locked identities remain locked rather than guessed | Zero V1 active and journal slots; every supported pre-V17 backup proves authenticated restore or is explicitly retired |
| Managed artifact markers | Store/key binding, operation/effect identity, target fingerprint, modes/digests, backup, activation/compensation state, and created directories | Authenticate backup AAD and rewrite marker metadata to V4 during open-time migration | Zero V2/V3 markers in active restore directories; every supported backup proves restore-to-V4 or is explicitly retired |
| Execution observations | Immutable V1 event bytes and their authenticated digest | Live ingestion accepts only the current contract; recovery may read V1 for projection/backfill without rewriting it as a current event | Zero V1 rows across every active `activity.db` and supported backup, or explicit backup retirement |
| Historical Oracle fixtures | Exact sealed launcher/schema/input evidence only | Never converted into current-product success evidence | Delete the fixture, schema/types, dedicated historical test, and registration together |
| Client-bundled ReleaseFacts | The one current manifest, registry, model data and cross-reference generated with the client | The daemon accepts only the manifest bytes pinned into the same client revision, then validates all resource digests and cross-references before exposing facts; there is no older production fallback | A future HTTPS catalog contract replaces the bundle with explicit download authentication, rollback and cache rules |

Current E2E and product smoke create current inputs. Historical Oracle and golden inputs stay in
their explicitly registered paths and cannot be cited as current-product success. MVP production
embeds the sole `release-facts/current` manifest, registry and model data in the daemon build. The
daemon reads those exact bytes directly, validates the registry/model-data digests and
cross-reference, and does not copy or read a second catalog under product storage; an invalid
embedded catalog fails closed and never falls back to another catalog. Deterministic compiler
inputs and bundle generation remain reviewable, but there is no detached signature, private release
key, signer approval, or metadata-only publication step. Future HTTPS delivery requires a separate
contract before it can replace this client-bundled boundary.
