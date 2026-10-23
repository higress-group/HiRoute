# Model connections feature

`ModelConnectionForm` is the MVP-12 domain component for preset API, custom API, direct-free, and API-key-free entries. Its host supplies one `ModelConnectionBackend` implemented through the native protected-input bridge and Client Core. The component does not register Tauri commands or mount itself in the global shell.

`ConnectionDirectory` groups the executable options into a complete branded provider
directory, with product selection, search and billing filters. The form owns the
service → configuration → model steps and editable source name. `ConnectionEditingSession`
retains each product's draft and credential only while that dialog is open; returning
between steps preserves it, while closing or saving releases unused protected inputs.
Saved-source edit and append enter configuration directly and retain their lifecycle rules.

The password field is uncontrolled. Its value is passed only to `registerProtectedInput` and cleared immediately; checks, previews, public state, and callbacks use the returned safe candidate reference. Editing the target, protocol, or authentication releases that reference and invalidates the prior check. Once Apply is dispatched, the component stops releasing the protected input because the save operation may have taken ownership, including when the response is uncertain.

Each edit has a client `edit_revision`; every check has a unique `check_id`. A result is rendered only when those values, the candidate lineage, and both returned copies of `input_digest` agree. Save uses the backend `candidate_revision`, maps the enable checkbox to `save_ready`, and uses `save_disabled` for a disabled draft. `complete` facts still pass through backend Preview and Apply authorization.

Explicit Custom API endpoints support HTTP and HTTPS for local, LAN and remote
services. Each HTTP field shows a plaintext notice and recommends HTTPS without a
confirmation gate. A missing directory path permits manual model check/save without
network probing. Closed target-validation reasons survive the daemon error envelope
as `message_key` values and become localized correction text; raw exception strings
and provider bodies never become UI messages.

The first inventory check selects no models. A tool check on an explicitly selected
model preserves existing choices by exact upstream model identity within the same
candidate and edit revision, using fresh selectable model references. Newly
discovered, removed, or unavailable models are never automatically selected.

The composition owner must provide the real Tauri/Client Core adapter, open/return navigation, current revision set, protected-input lifecycle, and save recovery callbacks. The feature intentionally does not implement storage, transaction recovery, routing compilation, or the “My models” page.

The connection-options response also carries the daemon-selected complete metadata snapshot. The directory uses executable native API options; model candidates and capability prefill come from the selected option and its matching metadata. Custom API prefill offers those same native options and keeps the configuration editable. Unknown capabilities use the form's editable defaults. Credentials are never inferred, lifecycle data is warning-only, and nested cost hints are explicitly presented as “not a price.” A connection check remains mandatory before selected metadata can be saved.

Exact registered prefill/check and new publication use the shared scoped-default resolver.
User/ambiguous declarations retain saved intent; connector-verified authority is preserved.
New UI saves use a V3 source/default transaction while released V2 requests and journal bytes
retain their original proof reader. The model detail default editor shows saved versus current
values and records official-follow/preserve choices through normal Preview/Apply.
