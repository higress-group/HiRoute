# DeepSeek Harness boundaries

[Code map](README.md) · [Worker context](worker-context.md)

DSH uses its official CLI and native ACP. HiRoute does not import a DSH SDK,
inspect npm package layout, or decode native history. An acceptance release is a
reproducibility input, never a production version allowlist.

| User capability | Owner | Required boundary |
| --- | --- | --- |
| Import existing explicit API sources | [Passive source reader](../../crates/integrations/src/agents/dsh_sources.rs), [shared candidate prepare](../../crates/daemon/src/control/runtime/model_connections/pi_discovered.rs) | Static public provider/model declarations, supported protocol and safe API-key reference. No OAuth, dynamic composition, credential helper or model call during discovery. Public output excludes endpoints and secrets; changed source evidence blocks Save. |
| Add model routes | [Native admission](../../crates/integrations/src/agents/dsh_native.rs), [YAML leaf](../../crates/integrations/src/agents/dsh_config.rs), [shared provider engine](../../crates/integrations/src/agents/additional_native/README.md) | Standard `profiles/web/cordis.patch.yml`; one provider per Plan with explicit Responses/Messages choice. Preserve native defaults and foreign configuration. Reject global `llm-pi-ai` shadowing. Restore stays in the shared conditional transaction. |
| Enable task routing | [Native local checks](../../crates/integrations/src/agents/dsh_native.rs), [shared settings](../../crates/application/src/agent_connection/settings.rs) | Actual Skill registry/loader, enabled read/bash modules, standard Skill roots and selected HiRoute CLI; check relevant home/Web/ACP resource overrides independently of model configuration. No model runtime/session prerequisite, additional validation button or automatic paid inference. |
| Execute a Worker | [Public patch renderer](../../crates/daemon/src/delegation/profile/dsh.rs), [ACP](../../crates/daemon/src/delegation/acp/mod.rs) | Selected native CLI, ACP initialize/new, explicit model option and finite prompt/cancel. Freeze route, budgets and per-run credential; keep resource HOME/DSH_HOME and custom Skill roots. Own task history, attachments and run patch separately. Disable auxiliary providers, telemetry and background shell promotion. |
| Continue | [Shared exact binding](../../crates/daemon/src/delegation/profile/native_history.rs), [ACP resume](../../crates/daemon/src/delegation/acp/mod.rs) | Require advertised `session/resume`, identical native/ACP session ID and frozen cwd/context/route. Native DSH interprets its history and must refuse missing or unusable state. Never replace the conversation with `session/new`. |

DSH composition layers replace whole module configurations. Model edits therefore
manage the standard Web profile only and reject an overriding home provider row.
The independent home default is also checked at planning, staging and activation;
removing its selected HiRoute model is blocked. Restore needs these static checks,
but remains available when the CLI is removed or no longer compatible.
Shared recovery copy directs the user to the selected client's model selector;
it must not assume Pi's `/model` command for DSH Web. The
[additional-route browser scenarios](../../apps/desktop/tests/v3/browser/agent-trust-scenarios.mjs)
reuse one fixture for this default-reference recovery; native persistence remains
covered by the real additional-model journey.
Worker launch supplies a final run-owned patch through public `--patch`; it leaves
the borrowed home/profile files untouched. JSON is used only as a public YAML
subset for generated patches. The bounded static reader accepts block/flow YAML
and JSON, while edits require a block sequence to preserve unrelated row bytes.
Executable tags, includes, merge keys and ambiguous rows are refused. Edit spans
are checked against semantic rows; a dash inside a multiline string cannot become
an owned patch row.

Passive discovery checks both the home and Web files; a home `llm-pi-ai` row
replaces the Web module, rather than merging providers. It imports only explicit
nonempty model declarations, not catalog-only providers or placeholder model IDs.
API-key environment values precede the private version-1 credentials refs. Resolve
the standard credentials module before reading a store: Web config precedes home
config, and each supplied config replaces the earlier object. Static absolute
`path` wins over `dshHome/.credentials.yaml`, then the selected DSH root's default.
Disabled/removed modules, replacement plugins, unsupported composition operations
and relative/tilde paths are refused; do not borrow HiRoute's cwd or HOME to guess.
Only the effective file needs private-file validation. Source
evidence binds home/profile bytes, the effective credential path/bytes and the selected environment secret;
Prepare and Save recheck that evidence without executing helpers or inference.
Provider/model compatibility options and custom authentication that cannot be
preserved remain ineligible. Only a nonempty explicit model `input` establishes
vision support; empty/missing input stays Unknown because native catalog/default
inheritance is outside this passive contract.
Run credentials stay in the environment via `apiKeyEnv`. For Messages, the
[Gateway ingress](../../crates/gateway/src/core_runtime/inbound_auth_tests.rs)
accepts the native `x-api-key` carrier only for a run-model token on `/v1/messages`;
the existing run authority still verifies the live run and frozen protocol.
Native account keys, model grants and control-run tokens cannot use this carrier.

The production capability checks use the CLI's public composition dump in a
private temporary DSH root, without activating model plugins. CLI version is
recorded for diagnosis. Module presence establishes only a local prerequisite;
actual ACP/model/history behavior is guarded on the operation that uses it.
Borrowed resource overrides cannot disable the required Skill/read/bash modules,
turn off standard Skill roots or substitute a different `dshHome`. Additional
custom Skill directories remain native-owned. The collaboration proof binds only
these resource configurations, so unrelated model-provider edits do not revoke it.
Custom plugins and unknown provider protocols need an explicit integration
contract; a successful local dump is not a claim that every custom composition
or future release works. No cross-version task migration or Continue guarantee
is made.

Start validation from [DSH product entry](../../tools/product-e2e/tests/dsh_delegation.rs)
and [run guide](../../tools/product-e2e/tests/DSH_INTEGRATION.md). The native context,
concurrency/cancellation, additional-model and main-Agent journeys are shared with
other ecosystems; [DSH fixture facts](../../crates/daemon/tests/support/dsh_native_context.py)
add public composition and opaque synthetic-history witnesses only.
[Native ACP fixture client](../../crates/daemon/tests/support/native_acp_client.py)
is a test consumer, not a production adapter. Required positive outcomes include
upstream request counts and protocol, independent tool artifacts, preserved user
files, exact Continue and real public CLI delegation. Fixture rejection or process
exit alone does not establish product completion.
The static-import journey reuses the common Scan → Prepare → Save → restart →
native-call oracle with a different valid default credential as a decoy. Select
`HIROUTE_PRODUCT_DSH_CREDENTIAL_SOURCE=path` or `dsh-home` to exercise either
override; rotation of the active store must reject the previous Save preview.

Public references: [CLI](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/cli/reference/README.md),
[providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md),
[ACP](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/acp/acp/README.md).
