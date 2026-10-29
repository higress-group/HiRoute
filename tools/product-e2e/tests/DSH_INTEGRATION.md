# DSH product acceptance

[Entry](dsh_delegation.rs) · [Code map](../../../docs/code-map/worker-context.md#deepseek-harness)

Select an explicitly installed official DSH CLI with `HIROUTE_WORKER_DSH_BINARY`
and an exact committed `HIROUTE_PRODUCT_CANDIDATE_SHA`. Native npm launch also
requires its supported Node on PATH. Do not select an adapter or unrelated SDK.
The executable test entry builds the matching production daemon and CLI and
requires every declared case to report green. An omitted installation is a
failure, not a successful skip.

Run the ignored `dsh_delegation` target through `scripts/validation.py backend`
with the feature plan and exact ref/SHA. The five product journeys cover passive
Scan → Prepare → Save → restart → use, both saved model protocols and independent
restore/credential rotation, native Skills and frozen exact Continue, concurrent
routes/cancellation/missing-history rejection, and a native main Agent reading
the installed user Skill before executing a real Worker through the public CLI.
Use `HIROUTE_PRODUCT_AGENT_PROTOCOL=responses` and `messages` for Worker variants.

For credential-import changes, run the exact
`dsh_static_api_discovery_imports_effective_source_and_rejects_stale_save` case
with `HIROUTE_PRODUCT_DSH_CREDENTIAL_SOURCE=path` and `dsh-home` separately. Both
variants retain a valid private default file with a different key, import only
the active store, reject Save after its rotation, preserve all native files, and
use the imported source in an ordinary native request. Zero inference during
Scan/Prepare/Save remains independently asserted. This targeted source case
does not replace unrelated Worker/Continue or native Desktop evidence.

All native homes and writable Web provider files are fixture-owned. Borrowed
HOME in production remains separate from task-owned history. Only the source
model is deterministic; native CLI, ACP, public control, Gateway, native tools
and Worker processes are real. The persisted Web model file is consumed through
native ACP `--patch` and explicit model selection, without injected provider or
credential values. This proves the persisted public configuration is usable;
it does not claim DSH's own Web UI was exercised. Desktop acceptance must also
configure and read back the real HiRoute UI and observe a real delegated task.

Native project journeys initialize their owned project as a Git repository.
DSH discovers project Skills from the nearest Git root, so an ancestor repository
must not determine the fixture's project scope.
Routing acceptance distinguishes native current input from trailing runtime
context carried in the same wire role. The installed middleware emits minimal
v2 purpose/session metadata before optional session, event or checkpoint reads.
Explicit compaction and session-title calls remain auxiliary if those reads fail.
The supported native GenerateOptions has no per-call headers: a managed provider's
existing Agent/grant headers gate the request-local final transport wrapper.
Its immutable context follows iterator creation, next, return and throw, including
nested and overlapping calls, without changing provider bodies or errors.
The root-loaded middleware listens across Cordis LLM service scopes. Native Web
session isolation must not filter it out: exercise both main/tool continuations
and session-title calls with the actual Web UI separately from ACP. The existing
managed-provider and Agent/grant transport gates still restrict metadata writes.

An optional window proof maps the current native turn/start activation's User
events to final wire User ordinals and binds the entire bounded User-envelope
digest sequence. It proves neither human origin nor completeness of old history.
Only native text blocks are projected with the supported adapter's within-envelope
join; later System/Developer-as-User envelopes stay in the digest scope, and
tool-result-only Messages envelopes have no User ordinal. Unsupported content or
a mismatch drops the proof and preserves the complete observed input. The optional
work reads at most 512 recent native events and bounds text to 2 MiB, 128 wire User
envelopes and 32 current positions. Old events outside that window do not invalidate
a correctly mapped new activation, and a missing proof still permits ordinary
classification, execution capture and assessment of actual prior output.

Late agent-message/subagent-settled and unknown source input keep their full native
payloads with uncertain task association. A sender session can be reused for a
different task; arrival in a newer parent turn does not make the old result belong
to it. Child initial task and return-guidance blocks remain one complete payload.
A runtime-context-only compaction continuation uses the normal temporary fallback
without fabricating a user or blocking model output. Long-history acceptance must
include more than 16 turns and more than 512 native events followed by fresh A/B
classification, capture and exact assessment membership.
Full restored YAML rows, including middleware registration, must match the saved
baseline before real ACP startup; absence of the adapter file alone is insufficient.
An adapter-absence test removes or disables the full native plugin registration.
A broken required import is a separate native installation failure. After Restore,
verify actual inference with the restored default and no model override separately
from successful startup.
For ACP, seed and prove that same default inference before installation. Its
`acp` provider/model configuration selects the initial route independently of
the Web `agent-default-model` row; a valid Web default alone does not establish
an ACP default. Retain both original baseline files and require their exact
restoration; omit model-selection RPCs during both default-inference checks.

ACP sends slash-command text as ordinary prompt input. Exercise real automatic
compaction through native context pressure, and distinguish explicit public SDK
calls from automatic native features. SDK fault, overlap and iterator fixtures
can run inside an owned native tool without changing the production middleware.
Native patch rows replace the complete plugin configuration: an overridden
`tool-subagent` row still needs its provider and tool name. A broken middleware
import may leave ACP available with a plugin-loader error; record the failed
installation and any inference availability separately.

The controlled main-Agent journey explicitly selects DSH's `danger-full-access`
mode for its fixture-owned commands, matching the Worker journey's `approve-all`
policy. It does not establish `workspace-write` sandbox support on the host.

The initial research/acceptance installation is `0.2.0-rc.2`. Record the actual
CLI version and full HiRoute SHA in each run. Production checks required public
capabilities, not this label. No multi-version matrix, native-history decoder or
cross-version exact Continue contract is introduced. Same-installation Continue
and daemon restart remain required. Controlled upstreams do not establish paid
provider/account compatibility.
