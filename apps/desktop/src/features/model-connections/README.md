# Model connections feature

`ModelConnectionForm` is the MVP-12 domain component for preset API, custom API, direct-free, and API-key-free entries. Its host supplies one `ModelConnectionBackend` implemented through the native protected-input bridge and Client Core. The component does not register Tauri commands or mount itself in the global shell.

The password field is uncontrolled. Its value is passed only to `registerProtectedInput` and cleared immediately; checks, previews, public state, and callbacks use the returned safe candidate reference. Editing the target, protocol, or authentication releases that reference and invalidates the prior check. Once Apply is dispatched, the component stops releasing the protected input because the save operation may have taken ownership, including when the response is uncertain.

Each edit has a client `edit_revision`; every check has a unique `check_id`. A result is rendered only when those values, the candidate lineage, and both returned copies of `input_digest` agree. Save uses the backend `candidate_revision`, maps the enable checkbox to `save_ready`, and uses `save_disabled` for a disabled draft. `complete` facts still pass through backend Preview and Apply authorization.

The first inventory check selects no models. A tool check on an explicitly selected
model preserves existing choices by exact upstream model identity within the same
candidate and edit revision, using fresh selectable model references. Newly
discovered, removed, or unavailable models are never automatically selected.

The composition owner must provide the real Tauri/Client Core adapter, open/return navigation, current revision set, protected-input lifecycle, and save recovery callbacks. The feature intentionally does not implement storage, transaction recovery, routing compilation, or the “My models” page.

The connection-options response may also carry the complete client-bundled current metadata snapshot. Registered options are shown only when the current registry has a native API option, provider-key authentication, and at least one exact model capability. Custom API users may select source-scoped provider/model records for editable prefill. Each field is gated by its `usable_for` scenario; unknown or conditional fields stay blank, credentials are never inferred, lifecycle data is warning-only, and nested cost hints are explicitly presented as “not a price.” A connection check remains mandatory before any selected metadata can be saved.
