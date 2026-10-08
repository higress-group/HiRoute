# Additional native model routes

The shared adapter owns one provider per selected Plan, identified by the context
namespace and model alias. The
application owns authorization, grants, publication and restore ordering; native
leaves own effective configuration, model budgets and result interpretation.
See the [Agent map](../../../../../docs/code-map/README.md) before changing owners.

| Native boundary | Qoder | Pi | DSH |
| --- | --- | --- | --- |
| Selected user target | `QODER_CONFIG_DIR/settings.json` | `PI_CODING_AGENT_DIR/models.json` (effective HOME fallback) | `DSH_HOME/profiles/web/cordis.patch.yml` |
| Model declaration / budget | [Qoder provider](../qoder_provider.rs), [Plan budget](../qoder_budget.rs) | [Shared provider renderer](../additional_native.rs), [Pi Plan budget](../qoder_budget.rs) | [Public YAML leaf](../dsh_config.rs), [shared budget](../qoder_budget.rs) |
| Separate default dependency | `model.name` in the target file | `defaultProvider/defaultModel` in user `settings.json` | `agent-default-model` in Web and home patches; home provider shadowing is refused |
| Ordinary Gateway authentication | Dedicated `/_hiroute/qoder/v1` bearer channel | `/v1`, fixed `X-HiRoute-Token` header; saved native auth may override ordinary Bearer | Fixed `X-HiRoute-Token` header; Responses base ends in `/v1`, Messages base does not |
| Actual native acceptance | [Qoder journeys](../../../../../tools/product-e2e/tests/QODER_DELEGATION.md) | [Pi journeys](../../../../../tools/product-e2e/tests/PI_INTEGRATION.md) | [DSH journeys](../../../../../tools/product-e2e/tests/DSH_INTEGRATION.md) |

Main-Agent configuration adds only selected Plan aliases. It leaves native defaults,
purpose routing, extensions, hooks, MCP and Skills to their existing owners. The
Worker uses a different transient profile with one frozen route and exact budgets.
A native provider name alone never grants ownership of an existing user entry.

## Shared transaction and native leaves

- [Native edits](../additional_native.rs) enforce the independent namespace,
  semantic ownership, default reference and encrypted restore binding.
- [JSONC edits](../qoder_jsonc.rs) preserve unrelated bytes and comments. The name
  is historical; it is the single bounded parser/editor for Qoder/Pi. DSH uses
  its static YAML leaf below.
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
  second mutation while the original operation has a pending tail.

The prepared local grant is written only to the private native provider, mode
0600. Public journals contain no bearer or original native settings. Status reads
use the protected artifact port; restoration can still remove an owned secret
following permission drift. Preserve existing Qoder journal/restore wire strings
as registered recovery contracts; Pi/DSH use their current closed kinds. Current
multi-provider writes produce V2 restore records; V1 remains a registered reader
for historical single-provider writes.

## Adjustment and recovery

Reconfiguration requires the original authenticated restore record and unchanged
owned provider semantics. Unrelated user edits survive. An untouched file returns
to its exact former bytes; an edited file loses only the owned provider member.
New untouched files are removed, and harmless empty containers can remain to
preserve comments. Selecting an alias as the user default blocks removal until
that reference changes; retaining the alias while rotating a token remains valid.
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

The [shared Agent/Worker map](../../../../../docs/code-map/worker-context.md#deepseek-harness)
locates collaboration and execution separately. The
[DSH acceptance entry](../../../../../tools/product-e2e/tests/DSH_INTEGRATION.md)
retains stale-source, credential override, ordinary saved-route and restore witnesses.
