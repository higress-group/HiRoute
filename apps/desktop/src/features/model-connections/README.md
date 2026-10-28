# Model connections feature

`ModelConnectionForm` is the MVP-12 domain component for preset API, custom API, direct-free, and API-key-free entries. Its host supplies one `ModelConnectionBackend` implemented through the native protected-input bridge and Client Core. The component does not register Tauri commands or mount itself in the global shell.

`ConnectionDirectory` groups the executable options into a complete branded provider
directory, with product selection, search and billing filters. The form owns the
service → configuration → model steps and editable source name. `ConnectionEditingSession`
retains each product's draft and credential only while that dialog is open; returning
between steps preserves it, while closing or saving releases unused protected inputs.
Saved-source edit and append enter configuration directly and retain their lifecycle rules.
The trial host can opt in to entering a single eligible product after a successful
scoped options read. Returning to the directory or interacting with it ends that
automatic choice for the current opening. The host's custom-entry permission is
unchanged. Per-product and per-prefill editable baselines distinguish real unsaved
changes and model selections from generated defaults and check metadata; current and retained
credential inputs are checked in memory when leaving, without serializing them.
Authentication None preserves the current product's typed Key in that owner without
using it in a check, whether reached directly or by swapping primary and additional
protocol endpoints. Invalidating a protected-only input releases it and retains a
visible re-entry requirement. Custom prefills retain separate editing contexts so a
Key cannot move to another product template. Current, retained and parked drafts all
compare against the same product-and-prefill baseline. A new prefill derives its
defaults from the prior context's baseline so carried user edits remain edits; checks
must not turn an existing user model declaration into an automatic default. Only an exact untouched
automatic blank model row is excluded from the dirty comparison; validation still rejects zero models.

The selected registered product is executable only after the latest successful
options read qualifies that exact option. The form retains its identity and edits
through removal, disabled qualification or read failure; Reload does not check it.
Provider details derive from the current directory and locale. A catalog-changed
recovery read belongs to its check, product, edit revision and page generation, so
later edits, cancellation, reads or closure cannot apply an obsolete response.

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
