# Decision connection product calls

The backend owns `contracts.ts`, its Rust Domain and Application API DTOs, and
the deterministic [host examples](../../../tests/decision-services-v2-contract.json).
The examples describe Product Session arguments and returned values; they do
not implement a provider or replace a production-path test. Both native Tauri
and the web console use the same argument wrappers. Empty reads pass `{}`;
save and test pass `{ input: ... }`. Successful calls return the illustrated
payload, not a daemon machine envelope. Host failures retain the existing
`DesktopFailure` native, transport or backend-envelope representation.

`services` contains the latest saved revision of each service. Each entry keeps
the exact untagged V1 or tagged V2 service and its canonical configuration
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
V2 call retains its response shape. Network policy/revision/environment identity
is separate from credential identity; independent/custom tests have no inherited
Native evidence. `preview_native_network_route` with a saved decision selector
reads the current safe route without DNS, Keys or provider calls. Only the displayed
service needs that read; growing unrelated services must not add provider work.
Compare the admitted network state with the current readback before retaining a
current pass. A failed current read makes that pass unverified, never current.
`InheritedSourceNetwork` exposes a compact summary and the source-owned editor;
source save refreshes metadata while preserving decision/parent drafts and never
starts another test. Collapsing retains the mounted editor and original pending
Operation. Proxy-authentication repair leaves provider-Key management untouched.
