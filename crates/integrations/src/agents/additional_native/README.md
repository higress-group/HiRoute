# Additional native model routes

The shared adapter owns one provider per selected Plan, identified by the context
namespace and model alias. The
application owns authorization, grants, publication and restore ordering; native
leaves own effective configuration, model budgets and result interpretation.
See the [Agent map](../../../../../docs/code-map/README.md) before changing owners.

| Native boundary | Qoder | Pi | DSH | OpenCode |
| --- | --- | --- | --- | --- |
| Selected user target | `QODER_CONFIG_DIR/settings.json` | `PI_CODING_AGENT_DIR/models.json` (effective HOME fallback) | `DSH_HOME/profiles/web/cordis.patch.yml` | Absolute `OPENCODE_CONFIG`, otherwise additive config directory or XDG config root; existing JSONC/JSON target |
| Model declaration / budget | [Qoder provider](../qoder_provider.rs), [Plan budget](../qoder_budget.rs) | [Shared provider renderer](../additional_native.rs), [Pi Plan budget](../qoder_budget.rs) | [Public YAML leaf](../dsh_config.rs), [shared budget](../qoder_budget.rs) | [Singular provider renderer/context](../opencode.rs), shared Pi numeric budget policy |
| Separate default dependency | `model.name` in the target file | `defaultProvider/defaultModel` in user `settings.json` | `agent-default-model` in Web and home patches; home provider shadowing is refused | Effective user-layer `model`, `small_model`, agent/mode models, provider overrides and allow/deny lists |
| Ordinary Gateway authentication | Dedicated `/_hiroute/qoder/v1` bearer channel | `/v1`, fixed `X-HiRoute-Token` header; saved native auth may override ordinary Bearer | Fixed `X-HiRoute-Token` header; Responses base ends in `/v1`, Messages base does not | Both SDKs use `/v1` and fixed `X-HiRoute-Token`; inert SDK API-key placeholder |
| Actual native acceptance | [Qoder journeys](../../../../../tools/product-e2e/tests/QODER_DELEGATION.md) | [Pi journeys](../../../../../tools/product-e2e/tests/PI_INTEGRATION.md) | [DSH journeys](../../../../../tools/product-e2e/tests/DSH_INTEGRATION.md) | [OpenCode journeys](../../../../../tools/product-e2e/tests/OPENCODE_INTEGRATION.md) |

Main-Agent configuration adds selected Plan aliases and, for Pi/DSH/OpenCode, one
bundled routing adapter. It preserves native defaults, MCP and unrelated Skills. The
Worker uses a different transient profile with one frozen route and exact budgets.
A native provider name alone never grants ownership of an existing user entry.

Native protocol selection and the upstream binding are separate contracts.
`plan_protocols[plan_id]` selects the native Responses or Messages provider through
the ordinary settings transaction. Changing only an upstream binding cannot make
native controls portable: for example, Responses
`include: ["reasoning.encrypted_content"]` cannot be expressed by a Chat Completions
target. The [request adapter](../../../../gateway/src/adapters/request.rs) rejects
that conversion before dispatch. A supported native Messages selection can produce
a different, representable request; it must retain the original session and actual
history, preserve the saved default, and pass complete Restore and native inference.
Do not remove requested controls or edit retained history to obtain compatibility.

A provider's remote MCP result does not establish execution of a declared local
function tool, even when its response says completed. Keep each protocol's actual
call/result identity and native execution evidence. Do not rename remote calls into
local functions, synthesize tool results, or retry an already delivered response
automatically. Verify the chosen model and protocol with actual native tool
continuation; successful text-only inference does not establish this capability.

[Routing adapters](../routing_adapters.rs) use protected file effects with exact
content ownership, borrowing, rollback and restore. Pi auto-loads
`extensions/hiroute-routing.js`; its native directory discovery skips `.mjs` files.
DSH inserts a public Cordis middleware next to the selected patch; OpenCode loads
`plugins/hiroute-routing.js` from its selected resource root; its automatic scan
also excludes `.mjs`. The hooks only annotate managed model
calls. Missing native identity keeps request scope; missing source proof retains
observed classification, capture and evaluation. Pi managed-provider
warming is disabled because it lacks a separate native request purpose. DSH reads
at most 512 existing event positions, and OpenCode's optional session lookup has a
50 ms deadline and a 128-message bound. Huge/unknown native histories may therefore
lose verified continuity without globally disabling smart routing. Restart/reload is needed for an existing process to load new
resources; installed files alone are not proof of live activation.

DSH's wire user role also carries runtime context. The adapter therefore binds
native `source.kind=user` events to exact serialized text positions; Gateway's
[optional semantic view](../../../../gateway/src/core_runtime/native_routing_view.rs)
checks the bounded proof before routing. It leaves the execution body unchanged.
The native pi-ai leaf concatenates pure text blocks within each user envelope;
the adapter verifies that projection before hashing the final wire blocks, without
joining separate messages or inferring user input from unknown sources.
Unknown content, missing proof or exhausted projection capacity uses the separate
observed lane with explicit source uncertainty. A relay/settlement following the
latest real input cannot acquire its proof merely by arriving in the same native
turn. The verified main lane records a capture gap rather than a complete-history
claim; later actual new inputs can recover full capture and evaluation.

Pi custom/unknown projected input remains observed; a known compacted continuation
requires saved input. Pending compaction ends at a new native user/checkpoint even
if its completion hook failed. OpenCode records prompt creation IDs and identifies
known compaction continuations through the pinned producer's typed part marker and
summary-parent relation. Overflow replay copies and unbound late child input stay
observed instead of acquiring the newer main task. A cold plugin without prompt
creation evidence keeps routing available; the next native prompt supplies it.
These mechanisms do not establish semantic scoring accuracy.

## Shared transaction and native leaves

- [Native edits](../additional_native.rs) enforce the independent namespace,
  semantic ownership, default reference and encrypted restore binding.
- [JSONC edits](../qoder_jsonc.rs) preserve unrelated bytes and comments. The name
  is historical; it is the single bounded parser/editor for Qoder/Pi/OpenCode. Only
  OpenCode enables native object/array trailing commas. DSH uses its static YAML leaf below.
  Ambiguous duplicate keys, invalid JSON and oversized files fail closed. Line
  comments accept LF/CRLF; standalone CR and Unicode line separators are rejected
  because native readers disagree about where such comments end.
- [Artifact effects](../additional_native_effects.rs) reuse protected snapshots,
  encrypted restore records, sensitive staging and conditional replay.
- [Typed additional-model facts](../../../../daemon/src/control/runtime/settings_facts/additional_model.rs)
  assemble only dependencies used by the model intent. Collaboration-only requests
  never read the native model file or its defaults.
- [Daemon activation](../../../../daemon/src/control/runtime/native_additional_model.rs)
  checks independent native dependencies during staging, preparation, final
  activation and retry. Preview alone cannot protect against a concurrent default
  change in another file. This guard covers Pi's **user** settings; project defaults
  are native project configuration, outside the global default ownership contract.
- [Budget admission](../../../../daemon/src/control/runtime/additional_model_budget.rs)
  keeps installed declarations compatible with publication changes and blocks a
  second mutation while the original operation has a pending tail. Only the exact active
  grant generation retained in the next publication owns a declaration. All affected
  Qoder/Pi/DSH/OpenCode contexts are reported with their Plan ID/alias, declared/proposed context
  and output, or a distinct missing/uncomputable reason. Sharing an upstream binding
  does not establish a Plan dependency.

Budget rejections use `hiroute.agent-model-budget-conflict/v1` safe facts. Admission
returns them as optional `error.model_budget_conflict`, leaving success `data`
absent so released typed envelope readers still work; an Install rejection freezes the same facts in
that step's existing terminal result and retains them through rollback/restart.
This diagnostic has no effect ownership or recovery authority. Qoder-only conflicts
keep the released Qoder code; other consumers use the released general conflict
category and `AGENT_MODEL_BUDGET_CONFLICT` terminal code. Error facts require
`PlanContentApplyRequestV2.include_model_budget_conflict=true`, which the Desktop
sets only for content publication. Omitted/false and generic preview/apply calls
retain the released closed error shape. This presentation flag is outside the
change spec and confirmation digest and never grants write authority. `operations.find` emits
these facts only when `include_model_budget_conflict=true`; omitted/false requests
retain the old strict response shape. Existing operation inputs, grant formats,
SQL schema and existing native settings formats are unchanged by this diagnostic.

The prepared local grant is written only to the private native provider, mode
0600. Public journals contain no bearer or original native settings. Status reads
use the protected artifact port; restoration can still remove an owned secret
following permission drift. Preserve existing Qoder journal/restore wire strings
as registered recovery contracts; Pi/DSH use their current closed kinds. Current
multi-provider writes produce V2 restore records; V1 remains a registered reader
for historical single-provider writes. OpenCode has its own current V2 restore
discriminator, retaining the same protected ownership and recovery engine.

## Adjustment and recovery

Reconfiguration requires the original authenticated restore record and unchanged
owned provider semantics. Unrelated user edits survive. An untouched file returns
to its exact former bytes; an edited file loses only the owned provider member.
New untouched files are removed, and harmless empty containers can remain to
preserve comments. Selecting an alias as the user default blocks removal until
that reference changes; retaining the alias while rotating a token remains valid.
For DSH, the provider restore baseline also retains the original ownership of the
Cordis adapter insert across unrelated edits and reconfiguration. Removing an owned
adapter file removes its owned insert; a genuinely borrowed file/insert survives.
A late dependency change blocks the file switch. A restore rejected before that
switch rolls back safely and needs a fresh operation. An operation whose service
activation was already sealed keeps its pending tail for retry.
Pending-tail retry requires the accepted dependency digest, including the original raw settings
bytes. A different safe default requires a fresh configuration operation; this
recovery guard does not silently authorize a revised user configuration.

The [public settings regression](../../../../daemon/src/control/runtime/settings_entry_qoder_model_tests.rs)
covers independent facets, permissions, grant secrecy, publication budgets and
late default changes. The shared [persisted-model product journey](../../../../daemon/tests/support/additional_model_product.py)
starts the actual native CLI from saved settings without provider/credential
arguments, rotates its grant, rejects removed routes, and restores model/Skill
facets separately. Unit fixtures do not prove native model calls. Neither native
protocol differences nor backend capability evidence require an extra user-facing
verification button or status badge.

## DSH static composition

The Web and official macOS Desktop entries have separate subjects and native
profiles. [Desktop admission](../dsh_desktop.rs) uses the app-owned Electron
carrier and public bundled CLI, with standard Desktop manifest validation; it
never changes the existing Web target or requires an npm CLI. Each profile owns
its adjacent routing adapter and native registration through the same transaction.
The bundled CLI cannot dump Desktop directly, so its standard Web dump is only
composition evidence. Actual Desktop loading and provenance need native acceptance.
Desktop Restore uses retained file ownership even when the app is gone; custom
Desktop bundle/dependency composition blocks new Configure. Passive source import,
Worker ACP and shared Skills remain under the existing Web/CLI entry.

DSH uses the same per-provider ownership, protected grant, stale-preview and
restore engine. Its leaf is [dsh_config.rs](../dsh_config.rs): bounded static YAML
patch sequences, one affected `llm-pi-ai` row, public `baseURL` and
`agent-default-model` references. Unrelated rows retain their bytes.
[DSH native admission](../dsh_native.rs) rejects a higher-priority global provider
row that would shadow the managed standard Web profile. It never evaluates JS,
includes, plugins or credential helpers. Its independent home and Web defaults are
checked during planning, staging, activation and retry; Restore retains these
static guards even when the CLI is removed or incompatible.

The reader accepts static block/flow YAML and JSON. Byte-preserving edits require
a block sequence; executable tags, includes, merge keys and ambiguous rows fail.
Edit spans must match semantic rows, including when multiline strings contain
text that resembles a sequence member. Shared recovery points to the selected
client's model selector rather than assuming Pi's `/model` command.

[Passive DSH import](../dsh_sources.rs) is a separate read-only capability. A home
`llm-pi-ai` row replaces the Web module, so providers are not merged. Import requires
explicit nonempty model declarations and preservable protocol/auth options;
missing model input remains unknown rather than inheriting unobserved catalog facts.
Environment API keys precede private version-1 credential references. For the
credentials module, Web configuration replaces home configuration; a static absolute
`path` precedes `dshHome/.credentials.yaml`, then the selected root's default.
Disabled/replaced modules, unsupported composition and relative/tilde paths fail
without guessing from HiRoute's working directory or HOME. Only the effective
credential file is validated/read. Prepared evidence binds configuration, active
credential path/bytes and the selected environment secret; Save rechecks it.
The single exact bundled routing insert is recognized passively by path and bytes;
other executable composition remains outside this reader's proven boundary.

The [shared Agent/Worker map](../../../../../docs/code-map/worker-context.md#deepseek-harness)
locates collaboration and execution separately. The
[DSH acceptance entry](../../../../../tools/product-e2e/tests/DSH_INTEGRATION.md)
retains stale-source, credential override, ordinary saved-route and restore witnesses.
