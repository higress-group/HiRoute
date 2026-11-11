# Decision connection product calls

The backend owns `contracts.ts`, its Rust Domain and Application API DTOs, and
the deterministic [current host examples](../../../tests/decision-services-v3-contract.json)
and unchanged [V2 examples](../../../tests/decision-services-v2-contract.json).
The examples describe Product Session arguments and returned values; they do
not implement a provider or replace a production-path test. Both native Tauri
and the web console use the same argument wrappers. Empty reads pass `{}`;
save and test pass `{ input: ... }`. Successful calls return the illustrated
payload, not a daemon machine envelope. Host failures retain the existing
`DesktopFailure` native, transport or backend-envelope representation.

`services` contains the latest saved revision of each service. Each entry keeps
the exact untagged V1 or tagged V2/V3 service and its canonical configuration
digest. `source` is the current shared source or null for independent/custom
services. `related_sources` contains the safe current summaries for sources
referenced by retained revisions, including the current source; it supplies
the source revisions needed to delete a whole service. An unavailable read is
not evidence that a source is missing. `missing` and `unavailable` preserve
the safe source ID while fields that cannot be established are null. Source
listing itself fails on a failed read rather than returning an empty success.

A `ready` source is enabled with at least one enabled configured Native API
Key. It is not an upstream access or test verdict. Null suggestion fields mean
unknown; a null codec must not be guessed. The opaque credential revision is
derived from authorization and current credential state, never Key bytes,
fingerprints or recoverable locators. A test reports the state admitted for
that call, so the UI can compare it with refreshed state after the call.

Save creates the next immutable service revision or deletes the service when
`service` is null. Include each affected source's observed revision exactly
once. Saving a shared revision affects only its selected source; switching from
A to B includes B alone because retained revisions still own A's unchanged
membership. Saving an independent or custom revision includes no source
revisions. Whole-service deletion includes all sources in `related_sources`.
`secret` is null for shared mode and deletion; a non-null independent
input is registered with protected input and never returned. A saved independent
header reference can be retained without re-entry only when its existing
authentication identity and permitted origin still match. A newly entered
secret produces a new protected reference. Switching to shared never copies
or accepts the independent secret. Save does not make an upstream request.
After acknowledgement, read back before testing the exact ID, revision and
configuration digest. Conflict and uncertain outcomes require refresh while
retaining non-secret edits; do not blindly resubmit.

Deletion shares general-model active-reference protection: non-deleted current
heads (including disabled routes), every draft, prepared versions, unresolved
holds/recovery and installed active/prepared/LKG publications block. Expired holds
block until reconciled. The server classifies keyed version/draft candidates and
independently checks installed publications even without a local version or index
entry. Failed or unknown reference reads never mean an empty set.
Unprotected historical versions, including deleted-route tombstones, do not block.
Removal deletes only saved service revisions and their consumer leases. Historical
Plan bytes, digests, reference indexes, tombstones and observations remain intact
for viewing/audit; restoration/execution is not guaranteed after removal.
All V1/V2/V3 writers check at stage and activation under the admission fence;
terminal finish retains revision/ownership and receipt checks without rescanning
history. Existing payloads, source/Secret effects and recovery remain unchanged.

The test response uses `DecisionFailureCode` for a failed synthetic provider
call. Request/host failures can also use the existing native, transport and
machine-envelope error codes. A generic upstream rejection is
`DECISION_INPUT_REJECTED`; it does not identify a wrong Key or workspace.
`decision_service` continues to select exact V1 content and
`decision_service_v2` selects exact V2 content. Neither codec changes the
frozen System One C v1/v2 logic, scoring or thresholds.

The network-aware `test_decision_service_v3` uses the same exact saved request and
one production diagnostic. Its strict V3 result wraps the unchanged V2 `check`,
optional admitted Native `network` evidence and a typed safe `failure`. The old
V2 call retains its response shape. Current hosts explicitly request
`include_decision_failure` for typed diagnostic details. The result schema and
presentation continue to recognize historical `budget_exhausted` and
`budget_unavailable` details. Shared decision tests and routed calls use the
source's eligible Keys for authentication. The Key allowance covers ordinary
inference. Network policy, revision and environment identify the inherited
route separately from credential identity. Independent and custom tests have
no inherited Native evidence. With a saved decision selector,
`preview_native_network_route` reads the current safe route without DNS, Keys or
provider calls. Only the displayed service needs that read. Growing unrelated
services must not add provider work.
Compare the admitted network state with the current readback before retaining a
current pass. A failed current read makes that pass unverified, never current.
`InheritedSourceNetwork` exposes a compact summary and the source-owned editor;
source save refreshes metadata while preserving decision/parent drafts and never
starts another test. Collapsing retains the mounted editor and original pending
Operation. Proxy-authentication repair leaves provider-Key management untouched.

The detail UI keeps configuration, credential and network digests internal to the
existing test-identity and freshness checks. Present the saved version, timing,
current test state and repair actions; failed-test diagnostics expose the typed
error code without opaque hashes. A standalone removal section follows the same
section/action/confirmation pattern as general models for decision models and
custom extensions. Opening or cancelling its confirmation
does not mutate a configuration. Confirmed removal retains the existing exact
revision/source-closure transaction and backend reference guards; a shared API
connection and its keys are not removed with the decision configuration.
The confirmation keeps its captured configuration digest, service revision and
related-source revisions. `preview_decision_removal` reads
`decision.removal.preview` with ID and expected revision and verifies the returned
configuration digest. Loading, changed, unavailable or mismatched responses
disable removal. Active blockers use existing
`DECISION_IN_USE` in the preview DTO and expose safe Plan/draft/revision identity
and protection reason; navigation is offered only for available current entries.
Installed-publication blockers may offer the existing authorized Update references
checkpoint. Opening/cancelling never performs that mutation. Ordinary history has
no reclaim action. Apply still uses `save_decision_service_v3`; a stale/revision
conflict triggers a fresh read-only preview. Product Session pins the existing
Apply revision set before resolving the captured configuration digest. This binds
the confirmation to the saved content even if its ID and revision are reused.
After a failed terminal response or a lost acknowledgement, read the original
Operation by its submission identity. A confirmed rollback refreshes blockers
before another confirmation is available.

Current hosts use `decision_services_v3`, `preview_decision_context` and
`save_decision_service_v3`. The preview takes the exact endpoint, model, codec
and context policy and returns the server-resolved request and per-question
windows, independent sources, metadata provenance and digest. It performs local
metadata work only. A native V3 save contains that exact policy and resolved
snapshot; a custom extension omits context. Metadata changing before acceptance
causes a refresh conflict, never an unseen replacement snapshot. Execution and
recovery consume accepted content without re-resolution.

Automatic uses exact maintained endpoint/model metadata or the conservative
32,000-token fallback. Custom requires a positive u32 request window, with an
optional positive question cap no greater than it. A request override preserves
an independent provider question cap. Automatic clears applied overrides while
the editor retains parked custom values per source/mode. TypeSafe automatic/off
is scoped to its exact official HTTPS System One destination; destination or
provider labels never authorize arbitrary headers. Preview responses are tied to
the exact draft signature and superseded reads cannot enable a different save.
Cross-field limit errors remain visible outside collapsed advanced settings.
A new question-limit conflict opens its field without moving typing focus; an
explicit repair action focuses the field and scrolls it above the sticky actions.
Inputs associate their error/help text for assistive technology. Invalid limits
do not offer a metadata refresh as a substitute for correcting the values.

Saving an old revision explicitly produces V3. Passive reads and opening an
editor preserve the old budget; active/prepared publications remain frozen until
explicit route selection and publication. Test freshness compares the entire
saved configuration digest, including context. The fixed connection test does
not certify maximum capacity. The old V1/V2 command entrypoints keep their shapes;
the V2 list excludes unrepresentable V3 latest records rather than stripping policy.
