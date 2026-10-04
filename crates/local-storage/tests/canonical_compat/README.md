# Default canonical records

Captured from unmodified pre-fix `CanonicalDigest::of` at
`94783c0e8ebacfdc3c5ba91f600d51a3d3f63cd6`, default serde_json/BTreeMap graph,
remote run `20260906-170130-eeb5ddf8` (capture test process_exit=0).
The JSON freezes an actually stored Operation journal, its exact operation_json,
revision and transaction digests, model-grant scope and its digest, and a stored
collaboration grant. All data is synthetic; no user credentials or stores.

Do not regenerate after the digest fix. The ignored capture test is a historical
recipe only. Its original JSON, plan/revision/scope digests and all six step proofs
are checked with default and serde_json/preserve_order dependencies. A separate
current-producer test preserves Operation identities and business inputs across
cold reopen, and verifies the original collaboration grant's positive and negative
authorization cases.

This pre-MVP agents.connect.apply sample lacks the complete current Plan/grant
facts. It is a frozen canonical fixture, not a production migration reader.
Injecting its journal into a current store must be rejected. Production
[startup admission](../../src/migrations/startup_format.rs) rejects unsupported
source formats (including schema22) before mutation. Historical migration
component tests do not establish a supported user-store upgrade path.

This fixture proves compatibility with the previous normal default graph; it is
not an inventory of user stores written by experimental order-sensitive binaries.
