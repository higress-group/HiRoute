# Connecting and managing models

## Purpose

A saved connection owns a stable source identity, protocol endpoints, credentials,
and selected model bindings. Templates and metadata assist configuration; saved
source membership and current runtime authorization determine actual access.

## Requirements

### Requirement: The API wizard uses the current executable directory

Desktop MUST present service selection, connection configuration, and model
selection as separate steps. Provider/product counts, search, billing filters,
and branding come from the complete current executable catalog rather than a
fixed sample. A provider with several products requires an explicit product
choice; custom, reconnect, and dedicated free-resource entry points may start at
the appropriate configuration step without expanding their catalog scope.

Every connection has an editable display name, initially the chosen product name.
The name is a label, not supplier/account/endpoint/model identity. New Desktop
names must pass its nonempty, 60-character, and duplicate-name checks; existing
same-name sources remain readable and independent. Check, model selection, save,
and recheck preserve the user's valid name without replacing it with a template.

Back/forward within the wizard preserves each product's separate non-secret draft
and current-session credential input. Switching products cannot copy a Key into
another product. Closing, discarding, or saving releases temporary credentials;
ordinary persistence never stores them. Editing check-relevant facts invalidates
the previous check. Internal navigation is not abandonment; close uses the normal
unsaved-edit rule. None of these actions implicitly starts inference.

For a new connection opened explicitly for one selected trial model, the caller may opt into a shortcut only after the options read succeeds and existing executable and model-scope filters leave exactly one product. It enters that product's normal configuration once per opening, with its actual name and an action to return to the directory. A provider with two eligible products is not unique. Loading, failure, unknown eligibility, an empty result, directory search or billing filters do not qualify. Returning or choosing a service prevents another automatic advance; asynchronous changes never overwrite an edited product. The ordinary directory and its permitted custom entry remain available, and this shortcut does not check, sign in, select a model, save or run inference.

#### Scenario: One exact trial product

- **WHEN** a new connection for an explicit trial model has one eligible product after a successful load
- **THEN** configuration opens once with the product name; the user can return to service selection without automatic check, save or inference

#### Scenario: Not unique or unknown

- **WHEN** two products share a provider, no eligible product exists, the read fails, or a directory filter shows one result
- **THEN** the user remains in the appropriate directory or error state; no shortcut selects a product

#### Scenario: Return and refresh

- **WHEN** the user returns to the directory or edits a selected product while options or language change
- **THEN** the user remains in control and inputs are not overwritten or silently switched

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/246#issuecomment-6461015079

### Requirement: Connection identity survives supported lifecycle edits

One template may create multiple independent connections even with identical
provider/model names or Key strings. Source and model views retain the connection
label and separate product identity. Rename, recheck, and append retain source
and existing Binding identity. Append adds only explicitly selected new models,
without duplicates, replacing credentials, disabling existing models, or changing
old facts after a directory refresh. Refresh retains the currently selected source.

Desktop and CLI MUST share rename, append, remove-model, delete-source, endpoint,
Key, and enabled-state semantics. Removing one model preserves remaining members
and shared credentials. Removing the last model requires explicit whole-source
deletion; removing the last Key instead leaves a repairable source. Unreachable
sources may be saved inactive, and failed replacement cannot overwrite a working
configuration with partially prepared state.

#### Scenario: Recheck reveals additional models

- **WHEN** a saved registered source's current account directory contains models
  not yet imported
- **THEN** they MUST be selectable for explicit append, while existing members,
  Keys, enabled state, label, and bindings remain stable.

### Requirement: Deletion respects exact references and owned secrets

Deleting or narrowing membership MUST inspect exact Bindings, including active
and disabled retained routes, held execution/Continue versions, drafts, and
unreconciled recovery references. Unknown or failed reference reads block deletion.
Concurrent references are rechecked inside the writer; previews are not authority.
UI shows the blocking objects and does not silently rewrite their plans.
Historical usage is preserved.

An explicit publication checkpoint MAY rotate old recovery references only from
the verified installed active publication. Preview binds its exact digest and
workspace revisions; Apply rechecks under the writer and advances publication
revision once through normal prepare/install/activate/rollback/restart. It preserves
plans, heads/tombstones, grants, aliases, and permission facts, replacing the old
last-known-good only after success. Empty active state remains NoNewCalls. The
checkpoint neither deletes a source nor invokes inference; unresolved/failed
checkpoint is not permission to delete or manually edit recovery snapshots.

Source deletion MUST atomically manage the source and its HiRoute-owned secrets
through a recoverable Operation. Compensation and retry preserve exact secret/source
identity and cannot overwrite later configuration. Subscription removal preserves
native login/configuration; late checks, saves, and maintenance cannot recreate the
removed source. A future scan requires explicit approval again. Released legacy
save semantics remain replacement semantics; append/delete are explicit intents.

Native API sources may also authorize versioned decision consumers. Ordinary model
editing, recheck, append and Key replacement MUST preserve registered decision
targets. Retained decision-service revisions and their publication/recovery
references participate in writer-side deletion checks. Removing a decision service
does not remove the API source, ordinary models or shared Keys; only unreferenced
decision destinations are removed through the coordinated recoverable change.
Decision sources remain distinct from subscription/OAuth login ownership. Their
complete lifecycle is defined in
[decision connections](../decision-services/spec.md#requirement-native-decision-models-may-reuse-an-existing-api-connection).

#### Scenario: Shared decision reference blocks source deletion

- **WHEN** a retained decision-service revision still references an API source
- **THEN** source deletion is rejected under the writer without removing shared Keys or unrelated model membership.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488572844

### Requirement: Raw upstream model IDs are opaque, not internal identifiers

Connection options MAY provide an explicitly requested, source-product-scoped
mapping from model configuration IDs to actual request IDs. It is a prefill hint,
not runtime observation or authority to assign catalog identity. The mapping SHALL
come from the current exact connector and endpoint-profile revisions and qualified
protocol endpoints. Ordinary callers that do not opt in SHALL retain the existing
response shape, including strict released readers.

CLI/API, persistence, compilation, publication, and credential authorization MUST
preserve an upstream ID exactly: nonempty, at most 512 UTF-8 bytes, no leading or
trailing Unicode White_Space, and no Unicode Cc controls. Unicode, punctuation,
and internal spaces are legal; U+FEFF is preserved because it is not White_Space.
No lowercasing, transliteration, normalization, or escaping may replace its identity.
Internal source/Binding/offer references retain their stricter identifier rules.

Desktop's manual entry policy currently accepts only ASCII for new or changed raw
IDs, with the same whitespace/control/byte bounds. Non-ASCII display names remain
separate. Existing, directory-selected, and metadata-prefilled non-ASCII IDs retain
their exact value when other fields are edited; replacing one with a different
non-ASCII manual value fails before check, inference, or save. A malformed legacy
model cannot break the entire management snapshot: retain readable state and
report that model's execution restriction.

#### Scenario: Rename a model whose existing ID is Unicode

- **WHEN** the user edits only its display name or capabilities
- **THEN** the original ID MUST survive unchanged and be sent exactly at execution;
  the manual ASCII policy cannot force a replacement.

### Requirement: Registered endpoint facts remain server-owned

Executable registered options MUST resolve from the client-version-bound ReleaseFacts registry and the selected validated official model snapshot, with embedded model data as the documented offline fallback. Manifest identity, digests, cross-references and supported schemas SHALL be checked before use. An arbitrary data-directory catalog, an unvalidated held copy or a schema suffix SHALL not become a selectable source of execution authority.

Connectors, endpoint/protocol profiles, authentication, secret slots, static headers, adapters, connection options and billing classes SHALL remain owned by the installed registry. A model binding becomes executable only when its exact references and required adapter are implemented, its offer matches the endpoint's entitlement/billing scope, and the existing source/account checks permit use. Descriptive-only models remain queryable. A known minimum compatible client SHALL be shown when available; otherwise the missing adapter or qualification SHALL be stated without inventing a version.

A registered check SHALL submit option identity, expected provenance, edit correlation, selected models and protected input. The daemon SHALL resolve exact destination, protocol, authentication, headers, identity and billing and reject client-supplied overrides of registered authority. Native API uses direct HiRoute adapters. Public endpoint hints may prefill a new editable connection, but a metadata refresh SHALL NOT rewrite a saved destination or expand Key permissions.

Saved source/binding identity, selected membership, credentials, target and historical admission provenance SHALL survive compatible metadata updates. A catalog-derived capability SHALL resolve from the current selected snapshot for new authoring/publication. A catalog digest change or a compatible change of context limit, tool/vision/streaming support or supported reasoning values alone SHALL NOT invalidate an otherwise legal saved connection. Existing user-declared values SHALL retain their source-local meaning and editable authority. Refresh SHALL neither fabricate a successful connection check nor overwrite historical save evidence.

New selection, check, recheck and publication SHALL qualify against one current snapshot. A withdrawn model, unsupported adapter, invalid source/account, or previously selected reasoning setting that is no longer supported SHALL produce a specific unavailable/conflict reason. The user SHALL be able to review a valid replacement; the service SHALL NOT silently change reasoning choice, drop selected members or publish a replacement Plan. Renaming a source, removing another member or changing Keys SHALL preserve unchanged saved model values and identity; newly added/rechecked members receive current admission. Withdrawal SHALL not prevent an otherwise legal removal/disable action.

Already published revisions SHALL keep their compiled model facts, candidate order and reasoning configuration. Their existing runs, new sessions, newly delegated Workers and Continue SHALL not depend on the mutable catalog or held copy, including after restart. Current authorization, cancellation and credential revocation still apply. Applying new capability defaults to execution SHALL use normal preview/publication. New logical requests through old publications still obtain current source-scoped valuation independently of frozen execution; settled history remains unchanged.

When a later client includes downloaded facts, the selected data revision and actual connection compatibility SHALL govern reuse; embedded presence alone SHALL not suppress newer official data or force a new connection check. A saved member that has no usable current official facts after withdrawal or cache removal SHALL report the actual new-use limitation, without erasing its identity or frozen publications.

#### Scenario: Existing registered model receives new limits

- **WHEN** a compatible official update changes a saved model's catalog-derived context limit while endpoint, account and adapter remain valid
- **THEN** the model remains selectable with the new default for authoring; source/binding IDs, Keys and membership are preserved and no connection check is claimed.

#### Scenario: Preserve manual and ambiguous capabilities

- **WHEN** a saved custom/native declaration has explicit or indistinguishable retained user values while catalog defaults change
- **THEN** those values remain intact; catalog refresh does not convert them into registered authority or discard the connection.

#### Scenario: Previously selected reasoning is removed

- **WHEN** a refreshed supported-reasoning set excludes the value selected by a new draft
- **THEN** new publication explains the unsupported selection and requires an explicit valid choice; the existing published revision retains its prior setting.

#### Scenario: New model uses an installed service

- **WHEN** the official catalog adds a model for an installed endpoint and the normal account/connection check succeeds
- **THEN** it saves and qualifies under the ordinary source lifecycle with the supplying snapshot recorded as historical evidence.

#### Scenario: Metadata is readable before execution support

- **WHEN** a model record requires an adapter the client does not implement
- **THEN** its metadata remains readable while executable selection/check/publication is unavailable with a specific reason and a minimum version only when established.

#### Scenario: Withdrawal and restart preserve published execution

- **WHEN** an official model is withdrawn or its only usable downloaded facts are removed, and the service restarts
- **THEN** new use requiring the absent facts is unavailable with a reason, while existing publications, new sessions through them and Worker Start/Continue use their frozen execution facts.

#### Scenario: Routine lifecycle edits preserve retained members

- **WHEN** the user renames a source, removes a different model or changes Keys after catalog capabilities changed
- **THEN** unchanged saved members keep their values and identities; only newly added/rechecked members undergo current admission, and removing or disabling an unavailable member remains possible.

#### Scenario: Upgrade absorbs a downloaded model

- **WHEN** a later client embeds a model already selected in a saved connection
- **THEN** the compatible connection continues without a forced recheck, and comparable data revisions choose defaults without replacing a newer usable cache.

#### Scenario: Public connection hint changes

- **WHEN** a publication changes a provider's suggested Base URL or authentication description
- **THEN** the new hint can appear in editable setup, but existing saved destinations, protected credentials and registered authority are unchanged.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295418452

### Requirement: Directory access and inference evidence are independent

Model directory reads are optional enhancements. They use the selected account,
retain valid items amid malformed entries, and distinguish unsupported, empty,
invalid, authentication, permission, quota, timeout, and connectivity outcomes.
A registered or manually declared supported endpoint can be saved without per-model
endpoint records, a directory, or inference probing. An unrecognized ID retains
raw/provider/source scope with no borrowed canonical configuration, score, price,
or free entitlement. Safe text fallback or user declarations are labeled untested.

Check/refresh is explicit and bounded, does not import automatically, perform paid
inference, cycle through unselected targets, or establish continuous health.
Cancellation stops further probe requests; late results cannot replace later edits.
A real model/tool test requires explicit invocation and proves only what it tested.
Protected save retains candidate correlation, version checks, and recoverable result
handling even if a check succeeded earlier.

#### Scenario: Directory is missing but the endpoint is configured correctly

- **WHEN** the account offers no directory or the directory operation fails
- **THEN** the user MUST be able to enter models/capabilities manually, preserve
  unknown fields, and save supported configuration without a synthetic inference.

### Requirement: Custom connections preserve protocol and transport scope

A Native API source may have at most one endpoint per Responses, Messages, and
Chat Completions protocol, sharing model selection and one Key pool. All endpoints
are unauthenticated or use that pool, though authenticated header forms may differ.
Each endpoint owns its full target, auth, and necessary static headers. Same-protocol
execution is preferred; an absent protocol may use only an expressible existing
conversion. Endpoint health is isolated; manual Key disable affects the source.

Custom configuration supports name, base URL/path prefix, None/Bearer/specified
Header authentication, and multiple manual models. A user may explicitly configure
HTTP or HTTPS for a loopback, LAN or remote endpoint, whether its host is a literal
address or a hostname. HTTP is not rejected solely because the destination is
non-loopback. HTTPS is recommended; an HTTP selection explains that API keys and
request/response content travel in plaintext without adding a mandatory confirmation.
Target normalization preserves the configured hostname; DNS and connectivity are
evaluated by bounded request transport, not by passive discovery or a syntactic check.
The same transport policy applies to durable source validation, publication and
actual Gateway execution. Registered catalog destinations remain bound to their
exact HTTPS authority; managed CPA transport retains its numeric-loopback scope.
Target changes require revalidation. Credentials cannot
be redirected to an unauthorized destination; TLS failure never falls back to plain
transport. Embedded URL passwords/query Keys must use protected input instead.

Vision, tools, context, output, streaming, native reasoning, and price may remain
unknown. Trusted facts, probe evidence, user declaration, and unknown are distinct.
Contradictory parameters or unrepresentable reasoning are rejected rather than
silently stripped. Missing canonical match, scores, or price alone cannot block
an otherwise expressible model. Cost hints without currency/unit/product/region/time
semantics are not PriceRate, free entitlement, or savings evidence. Metadata field
closure and reference pricing follow [model facts](../model-catalog-price/spec.md).

#### Scenario: Save an explicitly configured HTTP source without a directory

- **WHEN** the owner configures a valid LAN or remote HTTP endpoint, leaves the model
  directory path empty and supplies supported manual model declarations
- **THEN** connection check, save and saved-source recheck accept that target without
  DNS or directory/inference calls, preserve the exact target and authentication,
  and do not claim reachability, account access or inference has been verified.

#### Scenario: Correct an invalid connection target

- **WHEN** a connection check rejects an invalid URL, unsafe path, authentication
  header or protocol header, including an additional protocol endpoint
- **THEN** the daemon/API retains a closed, specific reason in the existing error
  envelope, with the released `INVALID_ARGUMENTS` machine code; Desktop presents
  a localized correction and retains input. Errors never expose URL credentials,
  protected keys or raw provider responses. Other URL/path/authentication checks,
  redirect restrictions and HTTPS certificate validation remain enforced.

Source Issue: https://github.com/higress-group/HiRoute/issues/154

### Requirement: Saved-source recheck reconstructs protected facts

Recheck submits source ID, exact expected revision, permitted candidate reference,
edit revision, and check ID. It MUST NOT accept caller-provided lineage, endpoint,
option, protocol, credential identity/value, or capability facts. Daemon reconstructs
registered sources from current catalog plus saved identity, and custom sources
from their private safe recheck descriptor. A legacy custom source lacking that
descriptor fails closed; connector-owned subscriptions use their reauthorization
workflow. Recheck shares deduplication, cancellation, candidate, save, and recovery
contracts and reveals no reconstruction secrets.

Native discovery/preparation/preview associations are process-local, not durable
permissions. The trusted process retains full effective configuration layers,
scanner/source revisions, credential descriptors, and catalog evidence. It rechecks
both complete evidence and protected credential fingerprint before Preview and,
after idempotency/version checks, before writer admission begins the Operation.
A changed high-priority endpoint/model invalidates a candidate even if the token
file is unchanged. Exact `prepare_id` replay returns the same candidate; reuse for
a different discovery conflicts. Prepare is synchronous with no cancellation
endpoint: closing drops UI continuation, and late results are discarded by edit
correlation. Restart requires new Scan/Prepare; admitted Apply/Result remains durable.

Executable resolution requires a regular executable and bounded process/version
handling, not a new owner/group-writable-parent trust rule. Discovery cannot run
native configuration helpers/hooks. Missing managed Codex runtime is
`runtime_unavailable`, not a successful empty scan; an available runtime without
native login may legitimately return a complete empty result.

### Requirement: Subscription modes retain their credential owner

Codex and Claude share check/select/save/disable/cancel/recovery workflows with
provider-specific readers. Each provider has at most one enabled account/mode.
Independent CPA sign-in is the default recommendation; NativeBorrowed is an explicit
option and saved choices do not migrate automatically. Login completion alone does
not enable routes or import all models.

Desktop subscription checks and model selection SHALL use subscription-specific
titles. Native discovery controls, including Hermes profile scanning, SHALL remain
on the device scan list and stay hidden in subscription details. Returning to the
scan list restores its controls; cancelling proxy editing retains the selected
subscription context.

A successful subscription save ends its Desktop check interaction. A later explicit
check, including another provider or account, MUST proceed without releasing or
invalidating the saved access. Running checks and verified checks that have not
completed saving retain their existing interaction ownership.

| Mode | Credential authority | Repair action |
| --- | --- | --- |
| NativeBorrowed | Native client owns login/refresh; CPA receives current access leases only | Update native login and explicitly recheck |
| CpaManaged | CPA owns an isolated private auth directory and independent OAuth refresh | Repeat independent sign-in as required |

Stable provider account identity, not email, filename, or auth-index, determines
matching. A same-account mode switch requires check/save and retains selected source
and unremoved models. Another account requires explicit check/selection. Pending
login is isolated from the enabled instance; saving reuses its auth directory with
one refresh writer. OAuth callback binds exact session/state/provider through
protected input, not logs or ordinary Operations. Cancel, expiry, restart maintenance,
and removal fence late callbacks and unsaved credentials. Disable revokes admission
and stops refresh while retaining credentials; remove-login first revokes saved
route access, then stops the exact instance and deletes its owned credentials.
Neither claims provider-global OAuth revocation.

### Requirement: Independent sign-in receives browser callbacks locally

CPA owns temporary browser callback listeners for the existing Codex
`http://localhost:1455/auth/callback` and Claude `http://localhost:54545/callback`
redirects. Bind only available IPv4/IPv6 loopback families. Each provider has at
most one pending sign-in; saved accounts do not retain a callback listener.

Browser and protected manual input MUST share atomic, one-use admission for the
exact provider/session/state. Invalid, stale and duplicate input cannot replace
an accepted callback. CPA reception permits retry after failed callback-file
publication. HiRoute does not resend an admitted manual code after a failed or
uncertain submission: observe status, or cancel and start a new sign-in. The browser
acknowledges reception without claiming token exchange or sign-in success.
Release listeners after reception, cancellation, expiry or failure, before slow
token exchange. Do not redirect or reflect callback secrets in browser responses.

A port conflict MUST close partial listeners and retain the same pending session
for manual completion. Desktop explains this mode before the user explicitly opens
the browser, and accepts the full return URL through the existing protected input.
Automatic reception opens the browser normally and retains manual completion as
a fallback. Successful sign-in still requires explicit Check/Save; it does not
change enabled-account or routing limits.

### Requirement: Borrowed subscription refresh cannot enlarge saved membership

Only approved, saved, enabled NativeBorrowed sources may maintain their association
to trusted CPA current in the same native context. Current access-token rotation
and capability revocation stay inside CPA; source qualification, Gateway Attempts,
and audit do not persist/compare that token generation. Stable source, selected
models, Bindings, and route references remain intact.

The latest successful pinned account directory constrains original selected models;
removed eligibility retains configuration as unavailable, and newly visible models
are not auto-imported. One independently sealed Codex connection may preserve native
models only when the user explicitly enabled it and the saved account, native
directory, and CPA pinned snapshot all prove those models. This connection-only grant
cannot enlarge general source selection, plan candidates, or another Agent's access.
Absent proof prevents a global provider switch.

Qualification converges through protected checks, management reads, or maintenance;
Attempt issuance does not discover online directories. Unobserved provider permission
changes may reach upstream and use its final rejection, without enlarging selection.
Unsafe credential reads, deletion, or stable account mismatch fail closed before
replacement credentials or capability issuance. Revision conflicts and transient
CPA/network failures retain configuration and report retry/repair. Deleted/disabled
sources cannot be revived by late work, and sent requests cannot be recalled.

An enabled saved source whose current CPA process has an empty directory after a
confirmed transient network failure SHALL recover through existing maintenance,
without requiring repeated user checks. Ordinary reads and Attempt issuance SHALL
NOT initiate this refresh. Maintenance may refresh once per cycle only after a
current, quiescent failure observation; it shares the original control deadline
and backs off repeated failures from five seconds to at most once per minute.
Authentication failures, revoked or replaced accounts, successful empty directories,
invalid catalogs and missing observations do not authorize this retry. Recovery
must recheck saved-source revision, account identity, controls, directory pins and
publication authority; it cannot synthesize membership or import new models.

### Requirement: Borrowed Codex auth follows its supported file semantics

An explicit `HIROUTE_CODEX_AUTH_SOURCE` selects that file; otherwise borrowing uses
the selected CODEX_HOME file store. keyring, auto, ephemeral, and unknown store modes
cannot fall back to stale `auth.json`; recommend independent sign-in. This path does
not read Codex Keychain, decrypt native stores, or reconstruct arbitrary launch layers.
Claude retains its supported file/macOS Keychain adapter; Keychain interaction is
limited to explicit checking rather than passive discovery.

Explicit `auth_mode=chatgpt` takes precedence even when an API Key is present.
Absent/null mode implies ChatGPT only when API Key/PAT/Bedrock login fields do not
select another mode. Current access token must be nonempty, bounded, and usable as a
request credential. `id_token` and `last_refresh` are not required for borrowing.
Account ID comes first from `tokens.account_id`, then the supported account claim
in ID/access tokens. Missing account identity requests re-login rather than using
email, user ID, or token hash. Parsing claims establishes local association, not
signature validation or upstream authentication. CPA-owned refresh credentials
retain their own required fields.

### Requirement: CPA isolation and revocation remain available during slow I/O

The packaged CPA locator binds the adjacent executable to its final signed-byte
identity; missing, changed, or nonexecutable CPA affects subscriptions only and
cannot silently use a user's different CPA. Native API and control remain usable.
Provider failures are isolated from each other.

Release Desktop builds MUST reject development-only, invalid or wrong-target CPA
manifests before producing an executable, using the same manifest policy as native
launch. Optional launch-time rejection MUST retain a closed diagnostic cause.
An absent CPA runtime SHALL remain runtime-unavailable in maintenance, model
management and routing rather than claiming that authentication failed. These
failures SHALL NOT rewrite saved source/model choices or block unrelated sources.

Credential/profile/file/Keychain/management-HTTP/process I/O MUST NOT synchronously
block Gateway async workers or hold their blocking locks. Bounded executors limit
running and queued work using the original absolute deadline/cancellation. Cancel
cannot release a slot while its blocking work still runs. Admission revocation is
separate from slow lifecycle serialization: disable/account/source switch revokes
old epochs before stopping old writers, and failed cleanup remains retryable.
Prepared results recheck source/account/token version/admission epoch; stale success
or failure cannot overwrite newer state. Final send rechecks capability epoch.

CPA-managed authentication may refresh and retry once for the same account/model
on 401 before response commitment. NativeBorrowed supplies no refresh token.
HiRoute does not implement or stack another OAuth refresh chain.

#### Scenario: Saved subscription runtime is unavailable after startup

- **WHEN** an authorized saved subscription candidate cannot prepare its CPA runtime and a later candidate in the same published plan remains usable
- **THEN** the Gateway skips the unavailable subscription and tries the later candidate within the existing order, deadline and cancellation boundary, without changing saved model choices or creating an upstream attempt for the failed preparation.
- **AND** a failed runtime directory read keeps subscription execution suspended while reporting temporary unavailability; it does not assert that the saved credential was revoked. Only saved-source recovery can restore execution, and disable, removal or shutdown keeps its revocation authority.
- **AND** rejected credentials, mismatched targets and unavailable shared authorization state retain their fail-closed behavior.

### Requirement: Proxy configuration is scoped and reports actual application

Subscription proxy modes SHALL remain inherited startup environment, direct and manual HTTP(S). Their desired configuration persists separately from applied process state. Manual mode overrides both cases of inherited proxy variables; direct clears them for that subscription scope; inherited mode preserves the host startup contract. Manual URLs cannot contain credentials and inherited secrets cannot enter status or errors. This subscription policy applies only to managed subscriptions and Claude subscription remote checks. Local management remains direct. Editing during scan retains results, cancel returns to them, and explicit subscription save/restart creates a fresh scan. An old running process or invalid configuration cannot be reported as applied.

Each saved Native API connection SHALL independently own a direct, inherited-service-startup-environment or manual HTTP(S) proxy policy, shared by its ordinary model bindings, protocol endpoints and explicitly associated shared-source decision consumers. Editing that source SHALL NOT modify another source, subscription policy, independent decision service, custom extension, Worker process/tool environment or system settings. A Worker request through Gateway to that source uses the source policy. Existing saved Native sources lacking new policy state and new connection drafts SHALL start direct. Imported native client configuration SHALL NOT implicitly adopt another process's proxies. Invalid/unknown new policy state SHALL fail explicitly rather than be treated as missing legacy state.

A successfully applied Native policy change SHALL affect subsequent requests without restarting the service or republishing route contents. Already admitted requests retain their exact network-policy snapshot; connections incompatible with the new route SHALL NOT be reused for new-snapshot work. Source/model/Key identities and saved model membership remain intact. The setting and its selected mode survive service restart. Inherit uses the service's captured startup environment, whose external change requires restart; it does not read a browser or current client shell environment. Native manual settings SHALL override inherited routing within the source scope and SHALL NOT mutate process-global environment. Manual URLs remain credential-free; protected inherited credentials never enter public views.

Management SHALL distinguish desired policy, actual applied snapshot and remote check evidence. Pending, failed or uncertain application cannot claim the desired snapshot is applied. Current-success evidence SHALL identify the policy/environment snapshot actually tested and become stale when that effective state changes. Application success does not assert provider availability. Normal revision guards, original Operation identity and recovery govern concurrent/uncertain saves. No scoped proxy failure authorizes an implicit direct internet retry.

#### Scenario: An external API uses a proxy while an internal API stays direct

- **WHEN** the user changes source A to manual proxy while source B and a subscription already exist
- **THEN** A and its explicit consumers use the selected policy; B, subscription settings and Worker tool environment retain their existing behavior

#### Scenario: Upgrade existing Native connections

- **WHEN** supported saved Native connections without the new policy state are opened by the updated service
- **THEN** their identities, model membership and credentials retain their meaning and their network path remains direct

#### Scenario: Apply while a stream is running

- **WHEN** a Native source policy is successfully changed while an already admitted stream uses the previous snapshot
- **THEN** the stream keeps that snapshot; subsequent requests use the new policy without service restart, automatic Plan publication or reuse of incompatible old connections

#### Scenario: Reopen and capture a new inherited environment

- **WHEN** a saved inherit policy is reopened after restarting the service with changed startup proxy variables
- **THEN** the selected mode persists, subsequent requests use the new captured environment, and old check evidence is not presented as validating it

#### Scenario: A save response is uncertain

- **WHEN** the client cannot confirm whether an accepted policy Operation activated the new snapshot
- **THEN** the UI preserves the original operation/recovery identity and desired-versus-applied distinction instead of claiming success or submitting a duplicate change

#### Scenario: Subscription settings retain their existing scope

- **WHEN** a user edits or cancels the subscription proxy panel during scan
- **THEN** cancel preserves scan results; explicit subscription save/restart applies that subscription policy without becoming the Native API setting

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/274#issuecomment-6499632874

### Requirement: Model management joins facts without creating a second authority

Search/filter/detail and operation actions MUST use shared exact source/Binding,
capability, reference-score, price, and subscription facts. Active route references
include regular and primary candidates in smart saving/custom branches, match the
Binding rather than name, and open the exact plan; disabled routes are not active use.

Saved subscriptions distinguish updating authorization, sign-in needed, connector/
network failure, and model exclusion from the latest pinned directory. They never
ask for an API Key to repair OAuth. Explicit recheck uses the complete existing
check/save/activation chain without reselecting membership or claiming inference.

Desktop uses `ListComputeV3`/`GetComputeV3` and
`hiroute.compute-management-snapshot/v3`; mode joins by source ID/revision from the
committed selection even offline. Unknown selection does not guess a mode.
Published `ListCompute`/`GetCompute` keep their strict V2 response fields from the
same projection; released CLI stays on that allowlist. The extension changes no
request envelope or persisted source and requires no implicit mode migration.

### Requirement: Model feed status and origin are visible and controllable

Model management in macOS Desktop, the Windows shell, web console and public CLI SHALL expose the same daemon-owned complete model metadata and feed state. Directory search/listing, model detail, editable prefill, capability views and price views SHALL use one selected effective snapshot per operation. Every maintained descriptive record SHALL be discoverable even without an executable binding or canonical match. Bounded pagination or another bounded complete-query path SHALL preserve snapshot identity and provenance; it SHALL not silently omit records to fit a frame.

Status SHALL disclose retrieval enabled/disabled, held publication sequence/issue time, selected official data revision and origin, highest accepted publication/data revisions, last retrieval attempt/result and last success, validation/recovery state, and counts that distinguish usable metadata from execution-restricted records. An updated existing model SHALL not be counted as ignored merely because it is embedded. Record/field views SHALL distinguish website/cache/bundled defaults and explicit user overrides, retain actual reviewed-as-of information, and name unknown or unsupported components. Publication, retrieval and model-provider release times SHALL not be presented as interchangeable.

Availability SHALL distinguish executable, needs a saved connection/qualification, unsupported or needs a known newer client, withdrawn/unavailable, and use by existing published Plans. Being used by a frozen Plan SHALL not imply availability for a new publication. Descriptive-only records can be inspected but SHALL not be offered as executable. Featured entries SHALL resolve against the selected catalog and retain their editorial role and actual origin.

Every host and the CLI SHALL support explicit refresh and the same persistent automatic-retrieval setting. Disabling automatic retrieval SHALL stop scheduled requests and invalidate an earlier in-flight result without discarding the selected usable cache; a later explicit refresh remains available. Removing the copy SHALL require a preview of changed defaults, saved sources affected in new use, embedded fallback and unchanged published revisions; only confirmation of that current preview may remove it. Retained acceptance floors and user overrides SHALL survive removal. The preview SHALL account for existing models whose defaults came from the website as well as feed-only models.

Public CLI help and schema discovery SHALL describe the shared views and controls. A first display of website-derived data SHALL disclose hiroute.ai retrieval and how to turn it off. Ordinary reads SHALL neither fetch metadata, call a model, perform connection checks nor mutate configuration. Concurrent reads SHALL report internally consistent snapshot identity or an explicit refresh/conflict outcome.

#### Scenario: Inspect a descriptive-only model

- **WHEN** the catalog contains a provider-scoped model with no executable binding
- **THEN** directory search and detail expose its metadata and limitations in the shared UI and CLI without enabling an executable selection.

#### Scenario: Inspect mixed default and manual origins

- **WHEN** an existing model uses website capabilities, a manual context limit and an unknown cache price
- **THEN** the view distinguishes these origins and states rather than applying one misleading origin badge to all fields.

#### Scenario: Same state across hosts

- **WHEN** the same service is opened through Desktop, Windows shell, web console or the public CLI
- **THEN** each reports the same selected snapshot and retrieval setting; hosts do not fetch their own catalogs.

#### Scenario: Remove a copy that updated an embedded model

- **WHEN** a held snapshot changed defaults for an embedded model used by a saved connection
- **THEN** removal preview identifies the fallback/default changes and any new-use restrictions, preserves manual values and explains that published revisions remain frozen.

#### Scenario: Turn retrieval off

- **WHEN** a user disables retrieval while a valid snapshot is held
- **THEN** all clients of that service show retrieval off, the copy remains usable, and a late in-flight fetch cannot reverse the choice.

#### Scenario: Complete bounded enumeration

- **WHEN** a caller enumerates a maintained catalog larger than one response page while a new snapshot activates
- **THEN** it can complete a coherent snapshot or receives an explicit refresh requirement; it is never told that a silently truncated or mixed-generation list is complete.

#### Scenario: Refresh updates only selected source facts

- **WHEN** a publication updates one provider while other records retain older evidence dates
- **THEN** the views show the new publication/retrieval time separately from each record's actual reviewed-as-of date.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295421495

### Requirement: Connection exit protects actual unsaved user changes

The connection editor SHALL ask for confirmation before exiting only when the session would lose user-created editable changes, an unsaved exact model selection or a temporary credential, including any retained product draft. Compare each product against its stable initial editable values; generated identities, edit/check revisions, check results and unedited drafts do not by themselves require confirmation. Reverting all editable changes and selections to their baselines removes the need for confirmation. An unsaved password or protected-input reference is protected without serializing or copying plaintext for dirty calculation. Changing products retains each product's own drafts and credentials without transferring a Key. The shared discard mechanism handles close, Escape and navigation once, and continuing preserves input and focus. Confirmed save with no later edits needs no discard prompt. A pending, admitted or uncertain save remains attached to its original Operation and its transferred credential is not released or resubmitted as a side effect of closing.

#### Scenario: Selection without edits

- **WHEN** a user only selects and browses products or automatic initialization creates defaults and IDs
- **THEN** closing or Escape does not prompt even when unedited product drafts were retained

#### Scenario: Real edits across products

- **WHEN** a user changes a name, endpoint, authentication, model declaration or selected model, or enters a Key, then changes products or a check fails
- **THEN** closing prompts once and continuing retains every affected product input

#### Scenario: Revert to baseline

- **WHEN** the user restores the original editable values and selection with no remaining credential or other edited product
- **THEN** closing does not prompt solely because a revision increased

#### Scenario: Save uncertainty

- **WHEN** an Apply is pending, accepted or its result is uncertain
- **THEN** the original Operation and protected-input ownership remain intact; leaving is not represented as cancellation or a repeated Apply

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/246#issuecomment-6461016031

### Requirement: Connection editing retains credential intent and current catalog authority

The connection editor SHALL retain an unsaved plaintext Key only in the current product’s existing in-memory credential owner when a same-context authentication or endpoint/protocol change removes or replaces the input. Restoring an authentication mode that needs a Key restores that product’s available plaintext; None neither sends nor uses the retained Key. Authentication or target changes invalidate the prior candidate/check and release a session-owned protected input that has not transferred to an Operation. If only a protected reference remains and its proof is invalidated, the editor SHALL retain an unsaved re-entry requirement and visibly request a new Key where needed, without revealing plaintext or using the stale proof. Product or prefill changes never transfer credentials; close and accepted or uncertain Operation ownership retain existing rules.

The editor SHALL compare current and retained editable drafts to the immutable automatic baseline for their product and prefill identity. A prefill and its initially provided models alone are not edits. A Key, changed name or endpoint, model selection, deleted or changed prefilled model, capability or other user input remains protected across context switches; restoring an existing draft, rereading metadata or running a check SHALL NOT reset its baseline or hide prior edits.

A selected registered product remains identifiable and its edits remain available when current options disappear, fail to load, or retain its ID but report it unavailable. The visible action and the actual check handler SHALL both require the latest successful executable qualification before registering a credential or checking. The editor never silently replaces the product. A current successful reload can restore eligibility, but performs no automatic check. Provider detail retains stable provider identity while names, products, availability and selection arguments derive from the latest directory and locale. Removal, addition, lost or restored provider and same-ID qualification changes SHALL NOT expose stale selectable options or force a different provider.

A catalog-changed recovery read SHALL share the current options generation and remain owned by the initiating check, product, edit revision and page lifecycle. Cancellation, product change, editing, newer reads or closure make its completion stale; a stale success or failure SHALL NOT change catalog, errors, phase or user input. A current recovery keeps an actionable retry and existing credential-release ownership.

Removing or recreating the exact untouched automatically generated empty-model placeholder is not an edit. This exception SHALL NOT mask a partial ID or name, a capability change, an existing or catalog-prefilled model, a selected model or another product’s edited draft/Key. Empty model sets remain invalid for check/save under the existing validation.

#### Scenario: Authentication changes with a typed Key

- **WHEN** a user enters an unsaved Key, changes authentication to None and then to Bearer or a custom header
- **THEN** the current product retains the Key in memory, None sends no Key, old check authority is invalidated, closing protects the change and another product never receives it

#### Scenario: A protocol switch changes authentication

- **WHEN** a user has typed a Key and promotes an additional no-authentication endpoint to the primary protocol, then returns to an authenticated endpoint in the same product and prefill context
- **THEN** the Key is retained only in that context, None does not register or send it, old check authority is invalidated, and immediate close or product change still protects the unsaved Key

#### Scenario: Current and retained prefill baselines

- **WHEN** a user selects an unedited prefill and exits immediately, switches products or saves another product
- **THEN** the prefill's automatically supplied values do not alone prompt, while an actual edit, pending selection, deleted prefilled model or Key is protected in the current and every retained context; restoring or revisiting it never overwrites its initial baseline

#### Scenario: Only a protected reference remains

- **WHEN** a protected input has replaced plaintext and an authentication or target change invalidates its proof
- **THEN** the old session-owned input is released once, the need to re-enter remains visible and protected, and no plaintext or stale proof is recovered

#### Scenario: A product loses or regains eligibility

- **WHEN** a selected product is removed, becomes unavailable under the same ID, the directory read fails, or eligibility later returns
- **THEN** the product identity and input stay visible; until the latest successful qualification both the action and handler reject before credential registration or check; recovery requires an explicit retry

#### Scenario: Current provider details

- **WHEN** the provider detail remains open while a product is added, removed or disabled, locale changes, or the provider disappears and returns
- **THEN** details and selectable values follow the current directory and locale; no stale product or different provider is silently selected

#### Scenario: Recovery read loses ownership

- **WHEN** a catalog-changed check starts a deferred reload and the user cancels, edits, selects another product, closes, or starts a newer read
- **THEN** the old success or failure cannot overwrite current catalog, error, phase or input and cannot release another owner’s credential

#### Scenario: Pristine placeholder

- **WHEN** a user opens manual model editing and removes the untouched automatically generated blank row
- **THEN** exit does not prompt solely for that placeholder, but partial user values, capability changes, real models, pending selections and other product credentials remain protected; zero models cannot be checked and saved

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/246#issuecomment-6462728885

### Requirement: Native API checks and execution share explicit network route semantics

Native API connection checks, configured model-directory reads, explicit inference/tool probes, production text inference, SSE and tool continuations SHALL resolve the selected source policy consistently for each exact original destination. Direct ignores ambient proxy configuration. Inherit uses the service's captured supported environment; manual uses the configured HTTP(S) proxy and bypass rules. Environment precedence and supported domain, IP/CIDR, port and loopback matching SHALL have one documented meaning across paths. A bypass match is an explicit reason for direct access, never a response to proxy failure. The check result SHALL expose a safe description of the route actually used and bind evidence to the exact policy/environment snapshot, target and credential authority.

A proxy SHALL be only a next hop. Preserve the original configured API URL, Host, protocol, upstream TLS identity, exact credential destination and current source/Key permission. HTTP and HTTPS upstreams and HTTP and HTTPS proxy endpoints are supported; HTTPS proxy and HTTPS upstream certificate verification remain appropriate to their separate peers. A provider Key SHALL NOT appear in CONNECT or Proxy-Authorization. Proxy routing that carries an upstream hostname SHALL NOT require successful local DNS for that upstream before using the proxy. Existing endpoint admission, forbidden redirects/header rules and protected input handling remain enforced.

Proxy dial, negotiation and TLS work SHALL be bounded by the existing request/check deadline and cancellation scope. A failed selected proxy SHALL yield an actionable, redacted transport/proxy error and SHALL NOT trigger direct upstream access or invented credential rejection. Subsequent candidate selection may use only the saved Plan's admitted candidates, their own policies, budgets and response-commit rules. Streaming remains bounded and cancellable. Policy, proxy/target identity and relevant TLS changes SHALL prevent incompatible pooled connection reuse.

Directory availability and inference evidence remain independent. An absent directory may retain the existing configured/manual-model check/save path without network probing; ordinary check/status/save SHALL NOT automatically send an inference request. A success from one policy revision cannot authorize or verify a changed route.

#### Scenario: Consistent check and production routing

- **WHEN** a configured source is checked and then used for text, SSE and a tool continuation under the same effective policy
- **THEN** each request uses the policy for its own exact destination; a directory success is not labelled as inference verification

#### Scenario: Explicit bypass

- **WHEN** the current supported bypass rules match the configured upstream destination
- **THEN** both check and production paths connect directly for that match and expose the bypass reason; a nonmatching destination uses the selected proxy

#### Scenario: Upstream DNS is only available through the proxy

- **WHEN** the proxy can resolve the authorized upstream hostname but the service machine cannot
- **THEN** the proxied request may reach that hostname without a mandatory local upstream DNS lookup while retaining the original Host/TLS/destination authorization

#### Scenario: Proxy is unavailable

- **WHEN** a selected proxy cannot be reached or refuses negotiation
- **THEN** the operation fails within its budget with a safe actionable error and no implicit direct upstream attempt

#### Scenario: Another saved candidate can execute

- **WHEN** a candidate has a proxy/network failure before the response-commit boundary and the saved Plan permits a later candidate
- **THEN** fallback obeys existing order and limits and uses the later candidate's own policy, without changing saved model selection

#### Scenario: Separate TLS peers and provider authorization

- **WHEN** an HTTPS API is accessed through an HTTPS proxy
- **THEN** both peers receive appropriate certificate/hostname verification and the provider credential is sent only in the authorized upstream request

#### Scenario: Optional directory and explicit inference

- **WHEN** a valid Native connection has no directory path and the user saves declared models without requesting an inference probe
- **THEN** the existing non-probing check/save path remains available and no network or inference success is fabricated

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/274#issuecomment-6499634214

### Requirement: Native API network management covers connection and trial journeys

Desktop, Windows shell, web console and the released CLI SHALL configure and inspect the same daemon-owned per-source Native API policy and safe desired/applied status. New registered/custom API configuration SHALL make network settings accessible before checking through an optional, collapsed-by-default section; manual mode provides proxy URL and optional bypass controls with actionable validation after expansion. Saved-source detail SHALL show network mode/application state, offer editing and explain the affected source models and shared decision consumers. Supported policy-only edits reuse saved credential authority without requiring a second API Key entry. Subscription settings retain their subscription-specific scope.

Saved-source recheck, model append and reuse SHALL retain the source policy. Scan/import permits explicit policy configuration and SHALL NOT silently adopt another process's environment. Trial new-connection/reuse flows consume the same behavior; a repair detour preserves selected model and parent draft. Network edits participate in semantic dirty detection and the existing close/back/cancel guards. Each product/prefill retains its own draft and protected-input ownership; policy edits invalidate dependent checks and stale responses cannot replace newer state. Concurrent changes and uncertain saves retain existing revision/operation recovery behavior.

The UI and CLI SHALL distinguish configuration/application facts, directory result, authentication outcome and explicit inference evidence. Reports identify the tested effective route without exposing credentials. Recognized proxy/timeout, proxy authentication, certificate, upstream authentication and directory failures receive precise safe explanations and relevant edit/retry actions; generic failures do not invent a bad Key. Reads and source selection SHALL NOT initiate provider calls. In a remote console, loopback proxy addresses and inherit mode SHALL explicitly refer to the service machine and its startup environment. View-only, loading and disconnected states preserve user intent and authority limits. Published CLI help/schema and the management Skill SHALL describe configuration, inspection, checks and the same semantics.

In Desktop, Windows shell and web-console connection editors, detailed network/proxy controls SHALL be collapsed by default for new connections and ordinary reopening of saved connections. A compact summary SHALL identify the current mode and expose a clearly labelled expand/edit action. Mode selection, proxy URL, bypass fields and detailed explanations appear after explicit expansion; the default direct check/save journey requires no expansion or additional network choice. An explicit network-settings or repair action may open the controls directly. Merely opening/closing the section SHALL NOT itself create an unsaved configuration change or perform a provider call.

Collapsing SHALL preserve the selected policy, unsaved values, semantic dirty state, credential ownership and actual saved behavior. An existing inherited/manual policy SHALL NOT reset to direct because its fields are hidden. Relevant applied/pending/error state remains visible in a concise summary. If validation rejects a proxy field, the UI SHALL reveal/focus the field or provide a visible correction action that does so; it SHALL NOT hide the only error explanation or leave a disabled primary action unexplained. Shared-source decision views SHALL show a compact inherited-policy summary and management action rather than duplicate an expanded configuration form.

A reopened connection draft SHALL apply an explicitly changed policy atomically with its checked connection edits through the same source revision and recoverable Operation, including a candidate-backed rename. Only a policy change on an existing source adds affected-consumer confirmation; ordinary new Direct setup adds no extra step. A standalone shape-valid policy repair SHALL NOT require a successful remote check. Cancelling its parent connection/trial/decision draft does not undo a successfully applied source policy, and the UI SHALL explain that independent save.

A route-only edit SHALL invalidate check/candidate authority while retaining same-context protected Key input and registered provenance. Temporarily inactive manual fields stay in the editor without being serialized into Direct/Inherit requests or blocking those modes. Typed safe failure phases SHALL drive repair actions; proxy authentication SHALL NOT request provider-Key replacement or disable that Key. Ambiguous transport failures retain neutral wording and retry/settings actions.

#### Scenario: Configure before first check

- **WHEN** a user adds a registered API or custom API on a proxy-required network
- **THEN** they can explicitly expand the optional network section, select network mode before checking, inspect the actual check path, select models and save through the existing protected flow

#### Scenario: Manage an existing multi-model connection

- **WHEN** a user opens network settings from one model belonging to a saved source
- **THEN** the UI identifies the shared source and affected models/decision consumers and edits its policy without changing model membership or unnecessarily asking for its saved Key

#### Scenario: Append using the saved route

- **WHEN** a user rechecks or appends models to a source with a manual proxy
- **THEN** the saved network policy is used and remains intact after the append

#### Scenario: Edit after a successful check

- **WHEN** a user changes proxy settings while an older check result exists or is still in flight
- **THEN** the old result cannot appear current or authorize the changed draft; genuine edits and credentials remain protected on close or product switch

#### Scenario: Repair a trial connection

- **WHEN** a trial flow enters network settings to repair its selected source and then returns
- **THEN** the selected model and enclosing draft remain available; refreshed state does not automatically call the provider

#### Scenario: Configure a remote service

- **WHEN** a browser connected to a remote HiRoute service selects inherit or enters a loopback proxy address
- **THEN** the UI explains that these refer to the service host; Desktop, browser and CLI expose the same saved and applied policy

#### Scenario: Read or save without inference

- **WHEN** a user reads policy status, selects a source or saves a valid configuration without choosing an inference test
- **THEN** those actions do not send an inference request or claim inference availability

#### Scenario: Routine direct connection needs no proxy interaction

- **WHEN** a user opens a new API connection editor with its default direct policy
- **THEN** proxy details are collapsed; a compact mode summary and optional expand action are available, and the user can check and save without opening the section or making another network choice

#### Scenario: Reopen a saved proxy connection

- **WHEN** a user ordinarily opens an existing connection configured with a manual proxy
- **THEN** details start collapsed with a compact manual-mode/application summary, and checks continue using the saved proxy rather than resetting to direct

#### Scenario: Collapse an edited network section

- **WHEN** a user edits proxy settings and collapses then expands the section
- **THEN** the draft values, selected policy, credential ownership and genuine unsaved-change protection are preserved; toggling alone neither changes configuration nor calls the provider

#### Scenario: Correct a hidden invalid proxy field

- **WHEN** a check or save rejects a proxy field that is currently collapsed
- **THEN** a visible error explains the issue and the affected field is revealed/focused automatically or through a visible correction action, without losing input

#### Scenario: Pending application remains visible

- **WHEN** a connection has a saved policy whose runtime application is pending or uncertain and the controls are collapsed
- **THEN** the compact summary exposes that state without displaying all configuration fields or claiming the desired policy is active

#### Scenario: One atomic reopened save

- **WHEN** an existing connection draft changes both a checked connection fact and its network policy
- **THEN** the source-CAS Operation applies or compensates both together after the affected-consumer review; a failed save cannot leave only one change active

#### Scenario: Route repair retains the current Key

- **WHEN** the user changes only the network policy and explicitly checks again within the same credential destination/context
- **THEN** a fresh check binds the new route without another Key paste, reusing no stale candidate and preserving registered provenance

#### Scenario: Cancel the parent after a source repair

- **WHEN** an in-place network repair succeeds and the user later cancels the parent trial or decision draft
- **THEN** the already saved source policy stays active while the unsaved parent draft follows its normal discard rules

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/274#issuecomment-6499635567

## Implementation references

- [Retain the selected trial model when reusing a saved connection](https://github.com/higress-group/HiRoute/issues/263).

- [Connection form ownership](../../../apps/desktop/src/features/model-connections/README.md)
  and [shared lifecycle planner](../../../crates/application/src/compute_management/mutations/lifecycle.rs).
- [Registered check](../../../crates/daemon/src/control/runtime/model_connections/registered.rs)
  and [source/reference persistence](../../../crates/local-storage/src/control/compute/management).
- [Subscription assembly](../../../crates/daemon/src/control/runtime/subscriptions.rs),
  [versioned query contract](../../../crates/application-api/src/compute_management_v3.rs), and
  [slow-I/O/revocation scenarios](../../../crates/cpa-bridge/src/runtime/tests/request_io.rs).
