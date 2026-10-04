# Qoder additional model routes

This adapter owns one `providers.hiroute-main-<context digest>` entry in the
selected Qoder user settings file. It adds the selected HiRoute Plan aliases to
Qoder's native model selector. It does not import native accounts/models or write
`model.name`, model overrides, purpose routing, hooks, MCP, or Skills.

See the [Agent capability map](../../../../../docs/code-map/README.md) for production owners and representative tests.
The native `providers` shape is tested against Qoder 1.1.65; its public CLI docs
direct users to the `/model` wizard. Treat an upgraded native release as a real
capability/verification boundary, not an automatically compatible configuration API.

## Responsibilities

- [Provider projection](../qoder_provider.rs) shares only the actual provider/model
  fields with the transient Worker/probe renderer. Main-Agent configuration omits
  the Worker's default model, fixed purpose mapping and extension restrictions.
- [Native edits](../qoder_native.rs) enforce the independent provider namespace,
  exact model budgets, semantic ownership and default-reference conflicts.
- [JSON-with-comments edits](../qoder_jsonc.rs) preserve unrelated bytes, including
  comments. Ambiguous duplicate keys, invalid JSON and oversized files fail closed.
  Line comments support LF and CRLF. Standalone CR and Unicode line separators
  inside line comments are rejected because Qoder's settings readers disagree on
  their interpretation; no foreign field may be hidden by that disagreement.
- [Operation effects](../qoder_native_effects.rs) use the existing protected
  artifact port for snapshot comparison, encrypted restore records, replay and
  sensitive staging. The application owns authorization, grants, publication and
  restore-first ordering; these functions do not start their own transaction.

`QoderFileConfiguration` receives the prepared connection grant only during
execution. Native `apiKey` contains that local bearer because ordinary Qoder
launches do not inherit a HiRoute token environment variable. The target mode is
0600. Status and reconfiguration use the artifact port’s private reader, which checks
0600 on the same opened file descriptor; restoration may still remove an owned
secret after a permission change. Public Operation payloads contain only the provider, endpoint, aliases and
budgets. Protected restore records must never enter public diagnostics or journals.

Qoder's provider supports fixed native authentication headers, not arbitrary
headers. Its persistent provider therefore uses the explicit local base path
`/_hiroute/qoder/v1`: standard bearer authenticates the current model grant on
`responses` and `models`. This entry reuses Gateway publication, execution and
receipts; it rejects unissued account tokens, run credentials and mixed headers.
The ordinary `/v1/responses` entry retains its separate `X-HiRoute-Token` channel.
Transient Worker and capability-probe settings still use `/v1` with their existing
run/challenge authority. Do not replace this distinction with a global bearer
fallback or a new proxy service.

## Adjustment and recovery

An existing provider cannot be adopted by matching its name. Reconfiguration
requires the preceding Operation's authenticated restore record and unchanged
owned provider semantics. Unrelated user edits survive adjustment and restoration.
An untouched file returns to its exact previous bytes; a changed file loses only
the owned provider member. A harmless empty `providers` container may remain to
preserve user comments. Newly created, untouched files are removed on restoration.

If `model.name` selects an alias being removed, Preview returns
`qoder.default.in-use`. The user must select a different native default first.
Keeping that alias while adjusting other routes or rotating the token is allowed.
Foreign changes inside the owned provider return `qoder.native.fields`; no user
value is included in that diagnostic.

The daemon must bind installed model declarations to the exact settings Operation
and grant, and apply the publication budget compatibility check. These static
native declarations do not refresh themselves when a Plan changes. The pure
`QoderTokenBudget` rule is domain-owned; `qoder_plan_token_budget` projects the
exact compiled Plan, including all active candidate/protocol bounds and existing
context/reasoning reservations. A positive compaction threshold is not a promise
that every prompt or Skill fits.

Backend ownership follows the existing settings transaction; start here when
changing a model route or diagnosing a parked native write:

| Behavior | Owner | Representative evidence |
| --- | --- | --- |
| Preview, independent facets and current installed declaration | [Qoder settings facts](../../../../daemon/src/control/runtime/settings_facts/qoder_model.rs) | Public configure / independent Restore in [settings journeys](../../../../daemon/src/control/runtime/settings_entry_qoder_model_tests.rs) |
| Protected native file execution and original Operation binding | [Qoder native effects](../../../../daemon/src/control/runtime/native_qoder_model.rs) | Journal secrecy, permission drift and conditional restoration in the same journeys |
| Installed budgets constrain publication admission and Install; parked tails retain Control ownership | [Budget and pending-operation guards](../../../../daemon/src/control/runtime/qoder_model_budget.rs) | Budget shrink rollback and exact parked-operation retry in the same journeys |
| Retry the original authorized settings Operation | [Settings retry](../../../../daemon/src/control/runtime/settings_retry.rs) | A second mutation is refused until the original tail completes |
| Real native session matched to Gateway receipts | [Qoder live checks](../../../../daemon/src/control/runtime/agent_live_check_qoder.rs) | [Ordinary startup product journey](../../../../daemon/tests/support/qoder_model_product.py), independent of Worker overlays |
| Persist the verified client surface without rewriting earlier results | [Current surface storage](../../../../local-storage/src/control/surface_check_tests.rs), [surface migration](../../../../local-storage/src/migrations/agent_surface_checks_v25.rs) | Every supported native surface is written through the real repository port; production startup retains earlier raw rows and the key |

## Representative checks

[Native contract tests](../qoder_native_tests.rs) cover preserving native defaults
and foreign providers, two selectable aliases, token rotation with unrelated edits,
conditional restoration, selected-default removal refusal, foreign provider
collision, and malformed/ambiguous JSON. [Plan budget tests](../qoder_budget_tests.rs)
retain the mixed-route, unknown-output and reservation contracts.

These tests do not prove native model calls. Product verification must launch the
installed Qoder against the saved settings without a transient provider/token
overlay, match its session identity to actual Gateway receipts, and preserve the
independent collaboration facet. Login and account capability remain Qoder-owned.
