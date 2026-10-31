# Credential pools

## Purpose

One model connection can own several ordered credentials. Configuration identity,
current credential eligibility, and per-model runtime health remain distinct.

## Requirements

### Requirement: Ordered pool edits are one explicit configuration change

A connection MUST support adding, replacing, removing, enabling, disabling, and
reordering Keys. Routing references the source, not a duplicate model per Key.
Replacement retains recognizable Key identity/order; failure retains the old value.
Combined edits stay in a draft until one atomic save of the complete intent.
Cancellation has no effect on the active pool. Enable/order controls support keyboard
interaction, and manual removal/disable affects applicable models in the connection
without promising to recall requests already sent.

Each logical request admits exact current Key IDs and order. An older frozen plan
may use the current source pool, but a later exact credential lease MUST recheck
current source/Key eligibility and generation. Request-local membership does not
freeze credentials or revive removed access; unrelated source membership cannot
be substituted.

#### Scenario: Replace and reorder, then cancel

- **WHEN** a user edits several Keys and closes the draft without saving
- **THEN** effective values, IDs, order, and enabled state MUST remain unchanged.

#### Scenario: Pool changes during an admitted request

- **WHEN** the request later leases a Key that was removed or replaced
- **THEN** the exact current lease check MUST enforce the new permission/generation;
  it cannot silently use a different Key outside that request's admitted sequence.

### Requirement: Automatic health is isolated by binding and credential generation

Before irreversible response delivery, a classified 429 or quota failure MUST cool
the exact failed credential and permit the next eligible Key in saved order.
Trusted Retry-After/reset guidance takes precedence and is not clipped by a local
cap; without it, quota cooldown is 60 seconds. Authentication failure disables the
exact credential. Credential state does not use Binding exponential backoff:
`transient_backoff_step` stays zero.

Automatic cooldown is scoped to source, model Binding, Key, and generation. The
same Key failing for model A cannot automatically cool model B; matching secret
strings cannot join different sources. Cooling Keys are skipped until eligible
again. Persistent failure follows the runtime's explicit disable/recovery result.
Management MUST distinguish user-disabled from runtime cooling/unavailable states.
Explicit recovery identifies its scope and reports real outcome; allowing a retry
is not proof of successful validation.

#### Scenario: One Key cools while another succeeds

- **WHEN** Key A fails with a retryable quota condition and Key B is eligible
- **THEN** the request MUST relay to B before changing models, retain the real
  attempts, and leave unaffected models/sources healthy.

#### Scenario: Every admitted Key is temporarily unavailable

- **WHEN** all remaining Keys are cooling
- **THEN** the request MUST use the bounded exhaustion/retry-time contract in
  [request routing](../request-routing/spec.md); UI timers cannot manufacture health.

### Requirement: Secrets and recognition masks have separate channels

Secrets MUST enter through protected input and stay under Secret storage authority.
Ordinary queries, previews, diagnostics, logs, and backup inventories cannot contain
secret values or recoverable credential locators. Failed updates have the same rule.
Saved Native Key recognition masks MUST be generated on the protected side from
exact source/Key/current-generation identity. A hash suffix is not a Key suffix;
short values never become fully visible. A user may show/hide a newly entered value,
but saved Keys provide no plaintext view/copy and do not return plaintext to Desktop.

Independent decision connections and custom extensions use a narrow Header Secret lease containing only the Secret
reference and the published legal header name. It MUST NOT acquire provider, model,
protocol, endpoint, account-pool enumeration, or ordinary candidate retry authority.
Gateway receives an opaque install capability for that exact header, not readable
or serializable secret bytes. The published endpoint supplies the destination trust
boundary; this does not add a separate destination-digest authorization system.

Native decision models that explicitly reuse an API source instead use an exact
decision credential lease. The source remains the sole Key owner; the saved service
revision, source lineage, registered decision target, consumer binding and current
Key generation constrain that lease. It MUST NOT use the independent Header Secret
path or register a fake ordinary inference model. The source's allowed destinations
include ordinary model endpoints and retained decision targets. Adding or removing
targets uses the recoverable source/service Operation and protected Rebind, retaining
the Key material and identity without plaintext re-entry. Ordinary source edits and
new/replaced Keys preserve this destination union. Decision automatic health is
binding-local and cannot silently cool unrelated inference models. See
[decision connections](../decision-services/spec.md#requirement-native-decision-models-may-reuse-an-existing-api-connection).

#### Scenario: A decision credential is used for another header

- **WHEN** a consumer attempts to install its narrow capability as a different
  header or ordinary provider credential
- **THEN** the operation MUST be rejected without exposing the credential.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488572844

## Implementation references

- [Current Native credential pool](../../../crates/local-storage/src/control/compute/management/runtime_pool.rs).
- [Protected recognition masks](../../../crates/daemon/src/control/runtime/model_connections/key_masks.rs).
- [Single-save model editing](../../../apps/desktop/src/features/models/model-draft.ts).
