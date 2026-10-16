# Discovering local model sources

## Purpose

Discovery identifies model configuration that HiRoute can safely prepare for
import. Discovery, explicit approval, saved source state, and real inference are
separate stages.

## Requirements

### Requirement: Discovery is passive and partial failures are local

A scan MUST leave native Agent configuration unchanged, avoid inference, and
return supported findings independently of unreadable paths or malformed items.
Executable trust and recognizable configuration are separate facts. Unsupported
endpoints, unavailable credentials, and helper/OAuth-dependent sources MUST carry
a precise importability state rather than a claim of verified access.

Codex subscription discovery uses the configured managed runtime. Registered
Claude configuration can be recognized independently of executable trust. The
supported Zhipu Coding Plan / GLM-5.3 Messages configuration must resolve its exact
Messages capability binding. An optional import to the corresponding official
Responses product is a distinct, explicitly selected source with exact source and
target facts; a second protocol must not make the original binding ambiguous.
Other API configurations use the custom connection path.

#### Scenario: One Agent's configuration is unreadable

- **WHEN** an independent source is valid but another scan item cannot be parsed
- **THEN** the valid source MUST remain discoverable; the failed item reports its
  own state and scanning does not execute its authentication helper.

### Requirement: Approval precedes credential materialization and saving

Only explicit user selection/approval may prepare or save a discovered source.
Protected preparation MUST bind the source and credential facts needed for the
operation; normal output cannot reveal recoverable credentials. Subscription
approval authorizes the stable local source to use the trusted CPA runtime, rather
than a single access-token generation. Scan, expansion, and cancellation do not
create credential material or maintenance rights. Disabled, deleted, and never
approved sources cannot be automatically revived. The current discovery workflow
does not add multi-account management.

A save MUST report actual per-source success. A source's common save unit is atomic;
failed replacement preserves the previous effective source. Retry of an admitted
Apply follows the original Operation and its idempotency/versions.

#### Scenario: Native token rotates after approval

- **WHEN** the approved native account refreshes an access token without changing
  the approved source/account identity
- **THEN** maintenance MAY refresh the same authorized source; it MUST NOT treat
  scan-only or disabled sources as having equivalent permission.

### Requirement: Codex subscription membership comes from the account directory

The managed CPA connection MUST discover account models from Codex's actual models
endpoint with its account, proxy, prefix, and exclusion rules. Static metadata may
fill fields for an exact returned ID but cannot introduce absent account members.
While creating an access lease, HiRoute MUST obtain the selected local Codex
engine's bounded version result and pass its major.minor.patch as
`hiroute_client_version`; Desktop's located engine and CLI's resolved executable
must not be replaced with another installation or an assumed latest version.

Registration/update, not ordinary inference, owns this lookup. Refresh has an
independent bounded 20-second control budget covering remote discovery and readback.
Missing version, timeout, or invalid directory cannot manufacture remote results:
a prior registered directory remains available, while a first failed lookup creates
no static substitute candidates. Discovery success does not prove inference.

#### Scenario: Directory lookup fails and later recovers

- **WHEN** an approved lease temporarily cannot obtain a client version or directory
- **THEN** failure MUST remain explicit and bounded; a later refresh uses the actual
  selected engine and current account results without inventing membership.

### Requirement: Native static API imports retain native precedence and evidence

Pi discovery MUST passively read effective user models/auth configuration and the
selected official package's static catalog. BOM/JSONC and native credential/template
precedence are preserved. A model-level `baseUrl` overrides its provider address;
provider defaults apply only where native semantics permit. Saved API credentials,
environment templates, and provider fallback are interpreted without running shell
helpers. OAuth, extra unsupported authentication headers, or unsupported overrides
remain non-importable. Provider and model identities remain visible separately.
DSH's static composition uses its own native reader through the same prepare/save
workflow; it must not be guessed from Pi rules.

The frontend submits opaque discovery/preparation identities. Daemon preparation
re-reads the native source and binds endpoint, raw model ID, credential, and native
source/catalog snapshot. Save rechecks those facts and rejects a stale source or
credential. Observed context/output/vision retain their observed basis; unverified
tool, streaming, or reasoning constants cannot be promoted to verified capability.

#### Scenario: Source changes between preparation and save

- **WHEN** the native effective address, model, credential, or relevant source
  snapshot changes after preparation
- **THEN** save MUST reject the stale candidate without modifying the source,
  and the user can rescan/prepare through the same protected path.

### Requirement: Discover configured Hermes model sources through existing scan flows

HiRoute SHALL add the selected effective Hermes profile as a native producer in the shared connection-point discovery and model-selection workflow. Its configured default main model and statically declared provider/model sources SHALL retain profile/native provenance, with exact resolvable product, endpoint, protocol and authentication-reference semantics. A user-named Coding Plan provider SHALL be recognized by the supported endpoint/product facts rather than requiring a built-in provider name. Configured native models, catalog candidates and unobserved live selections SHALL remain distinct. Supported Hermes findings SHALL use the same connection-level preparation and explicit model save as other supported scanners; unresolved/helper-dependent findings retain precise reasons and cannot silently become importable or recursive HiRoute upstream sources. The shared model-discovery passive/protected contracts and connection-point selection requirement remain authoritative.

#### Scenario: Produce a Hermes connection point with the configured default

- **WHEN** a selected Hermes profile configures a default model and a statically resolvable provider, including a user-named Coding Plan provider
- **THEN** the shared scan reports the scoped connection point and marks its native configured default; models offered by its prepared candidate retain their separate native/catalog provenance.

#### Scenario: Use shared model selection for Hermes

- **WHEN** the owner prepares a supported Hermes connection point and chooses its models
- **THEN** the generic CLI/Desktop preparation and exact-model save flow is used, including shared initial-selection and stale-fact semantics, without a Hermes-only import wizard or backend.

#### Scenario: Keep unresolved native state local

- **WHEN** Hermes contains helper/OAuth-dependent, unreadable, ambiguous or managed HiRoute entries
- **THEN** valid findings remain available, unsupported entries carry precise reasons, scan does not change native configuration or execute helpers, and no recursive upstream is admitted.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6169260806

### Requirement: Discover connection points and choose models through one shared workflow

HiRoute SHALL organize supported native API model-source discovery by connection point and expose the same scan, preparation and model-selection contract to CLI and Desktop. A connection point SHALL retain its native scope/provider provenance and exact product/endpoint/protocol/authentication binding; models MAY share a point only when those semantics are proven equal. Equal URLs or provider labels alone SHALL NOT merge different credentials, products, protocols or profile scopes. Scan SHALL remain passive under the existing discovery contract. After explicit connection-point selection, preparation SHALL expose all supported native declarations and eligible existing catalog candidates for that exact point with their fact basis, selectable state and unsupported reasons; a configured default alone SHALL NOT truncate the candidate to one model or become proof of account entitlement. The discovered-connection Desktop flow and canonical CLI management guidance SHALL initially select all models marked selectable in the current prepared candidate, while an explicit model subset takes precedence. Every save SHALL freeze a nonempty exact selection of model references under the existing protected Preview/Apply contract; scan, preparation, catalog refresh and defaults alone SHALL NOT save models or silently expand an accepted selection. Existing supported discovery callers retain their published shapes and meanings through the applicable versioned boundary.

#### Scenario: Group models under the right connection point

- **WHEN** multiple native model declarations share a proven scope, provider definition, product, endpoint/protocol and authentication binding
- **THEN** one discovered point carries their provenance and prepares a multi-model candidate; differing credentials, products, protocols or roots remain separate even if URLs or labels match.

#### Scenario: Offer endpoint models beyond the configured default

- **WHEN** a native default names one model but that selected point has other supported declarations or exact-product eligible catalog bindings
- **THEN** preparation exposes those models with their individual fact basis and selectable reasons; it does not claim they were configured or authorized by the account merely because they appear in a catalog.

#### Scenario: Start with all currently selectable models and permit a subset

- **WHEN** the owner starts a discovered-connection setup in Desktop or follows the canonical CLI guidance for a selected point
- **THEN** the initial selection includes only that candidate's selectable models; the owner can choose a subset, and the save Preview/Apply contains the exact selected references rather than a backend wildcard or omitted-selection default.

#### Scenario: Do not expand a pending or saved selection

- **WHEN** a later scan/catalog refresh adds a model or changes source/credential facts
- **THEN** previously selected/saved models are not automatically expanded; changed binding facts require the existing fresh-preparation/conflict handling, and adding models uses an explicit new selection through the existing lifecycle.

#### Scenario: Use the same protected preparation from CLI

- **WHEN** an authorized local CLI caller selects an opaque discovered connection-point reference for preparation
- **THEN** the backend rereads its scoped facts and prepares credentials privately under the existing local/protected-input contracts, returning only a safe candidate and model references for explicit save; no native helper or model call occurs merely because it was scanned or prepared.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6171346891

## Implementation references

- [Native discovery assembly](../../../crates/daemon/src/control/runtime/model_connections/pi_discovered.rs).
- [Pi filesystem scenarios](../../../crates/integrations/src/agents/filesystem_tests/pi.rs).
- [Subscription lifecycle](../../../crates/daemon/src/control/runtime/subscriptions.rs) and
  [borrowed Codex lease tests](../../../crates/cpa-bridge/src/borrowed_codex/tests.rs).
