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

The initial research/acceptance installation is `0.2.0-rc.2`. Record the actual
CLI version and full HiRoute SHA in each run. Production checks required public
capabilities, not this label. No multi-version matrix, native-history decoder or
cross-version exact Continue contract is introduced. Same-installation Continue
and daemon restart remain required. Controlled upstreams do not establish paid
provider/account compatibility.
