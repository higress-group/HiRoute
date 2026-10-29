# Contract convergence and compatibility

Every production boundary has one current write or ingestion contract. Older bytes may remain
only for an explicit recovery reader, a registered legacy request/result adapter, a separately named frozen fixture, or a rejection test. A
legacy decoder authenticates the original bytes before conversion and never becomes a second live
producer. A `legacy_adapter` preserves supported caller bytes, authorization and response/cursor semantics while normalizing at the boundary into the one current business model; it has an owner, exact paths and a support-policy removal condition.

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
schema V32 from the single production declaration
`LATEST_SCHEMA_VERSION` in `crates/local-storage/src/migrations/mod.rs`, and retains the
unversioned `control.sock` endpoint. The schema gate accepts exactly one plain, unattributed
top-level constant declaration in that source. Conditional declarations (including nested
`cfg`/`cfg_attr` expressions), module/macro bodies, comments and string literals cannot satisfy
it. Duplicate declarations and malformed lexical input fail. Inner file attributes require an
explicit gate review because they can condition the entire module; the checker does not evaluate
Rust configuration expressions. Matching literals in fixtures or other files cannot satisfy the
gate. This narrow lexical check is not a replacement for Rust compilation.

The local-storage owner owns both the schema and startup support policy. The current startup
path accepts a fresh store or the current stable storage format, with supported released sources admitted through the existing three-store migration/backup coordinator as defined by [release contracts](release-contracts.md) and `crates/local-storage/src/migrations/startup_format.rs`. The version pin is an explicit review
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
| Managed artifact markers | Store/key binding, operation/effect identity, target fingerprint, native permissions, backup, activation/compensation state, and created directories | Unix emits V4 and authenticates/migrates V2/V3 recovery. Windows emits Windows V5, binding the actual owner/DACL/read-only facts to its fingerprint and encrypted backup AAD; it never invents POSIX permission bits for recovery | Zero V2/V3 Unix markers in active restore directories; every supported Unix backup proves restore-to-V4 or is explicitly retired. Windows has no previously released producer or legacy Windows recovery format |
| Execution observations | Immutable V1 event bytes and their authenticated digest | Live ingestion accepts only the current contract; recovery may read V1 for projection/backfill without rewriting it as a current event | Zero V1 rows across every active `activity.db` and supported backup, or explicit backup retirement |
| Historical Oracle fixtures | Exact sealed launcher/schema/input evidence only | Never converted into current-product success evidence | Delete the fixture, schema/types, dedicated historical test, and registration together |
| Client-bundled ReleaseFacts | The one current manifest, registry, model data and cross-reference generated with the client | The daemon accepts only the manifest bytes pinned into the same client revision, then validates all resource digests and cross-references before exposing facts; there is no older production fallback | Installed registry/authentication/adapters and free eligibility remain authoritative. A selected complete HTTPS snapshot replaces descriptive model/default facts as a whole; embedded facts provide fallback |

Current E2E and product smoke create current inputs. Historical Oracle and golden inputs stay in
their explicitly registered paths and cannot be cited as current-product success. Production
embeds the sole `release-facts/current` manifest, registry and model data in the daemon build.
Invalid embedded facts fail closed. The HTTPS [model feed](../assets/model-feed/README.md) replaces the complete official model-data snapshot, including descriptive-only records, rating identities and scoped documented prices. It cannot replace registry/authentication/adapters or free-offer eligibility. New authoring captures one semantic generation; published execution and settled valuation remain frozen. Publication sequence is independent from semantic revision/digest.

Control schema 31 adds `compute_source_defaults` with a source foreign key, plus before/after default intent on the existing management effects. Source revisions/CAS and the same Control transaction own activation and compensation; there is no second preference journal or revision stream. V3 change/mutation proofs authenticate the original V2 source mutation before normalization. Supported V2 request and operation bytes retain their original digest and serialization. Full SQLite backups include the preference table; current-storage validation checks its source/model reference closure. Observation's storage format is unchanged.

Trust reports, threshold replay and routing timelines project retained execution evidence without
rewriting it. Added query/command contracts are registered as current boundaries. The previous-routing
query consumes existing typed Agent journals and performs a normal settings edit; it introduces no
second restore journal or persistent trial state.

Control schema 32 establishes authoritative unresolved-operation membership and
finalization coverage in the existing coordinated migration transaction. All current
journal writes preserve the membership/count with the same transaction; missing
coverage is not an empty recovery set. New operations reference separately stored
immutable input and large compensation payloads, authenticated on cold reads and
exactly checked on warm writes. Supported inline journals retain their original
bytes and recovery writer; conversion does not rewrite unrelated completed history.
Ordinary startup verifies current/prepared/LKG publications, current/held Plan
versions, active Secret dependencies and unresolved work. Full-store audit and the
initial conversion are separate work and do not imply unchecked history is healthy.
Runtime and Secret stores join the same schema32 coordination with no independent
migration path. Observation's format is unchanged.
