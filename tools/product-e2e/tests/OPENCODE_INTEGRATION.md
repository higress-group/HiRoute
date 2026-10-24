# OpenCode native integration acceptance

OpenCode uses the shared Agent settings transaction, static API import and native ACP Worker lifecycle. This guide owns the native acceptance inputs; the [Worker map](../../../docs/code-map/worker-context.md) locates production code.

## Installation and context

Select the actual native executable explicitly. The official `opencode-ai` npm launcher can locate a platform package without being executed. An ambiguous platform/CPU variant requires choosing its native binary. Initial Worker format admission supports the official v1.19.3 platform package with adjacent name/version/os/cpu/libc metadata; a standalone copied binary or an unaudited format remains unavailable for Worker/Continue. Discovery, ordinary model settings and Skill ownership do not depend on this database-format capability.

Use a dedicated synthetic HOME and workspace for acceptance. OpenCode uses native HOME/XDG config/data/cache/state semantics, absolute `OPENCODE_CONFIG`, additive `OPENCODE_CONFIG_DIR` and an absolute frozen database selector. Do not use daily credentials or move a real user's history. The production Worker borrows these roots; its owned task directory contains only descriptors and run material. Existing database migration IDs must match the audited native format before launch. Unknown/pending/legacy formats are preserved and refused.

Windows npm discovery accepts the official declared `opencode.cmd` entry, including
an extensionless PATH lookup, through the shared protected producer reader. It never
executes the shim or adds a Node dependency to native launch. Wrong producers,
reparse paths and ambiguous platform packages remain unavailable.

Saved-model reference checks include bounded Markdown frontmatter from the selected
user `agent(s)/**/*.md` and `mode(s)/*.md` resource directories. The existing
Preview/staging/activation/retry dependency digest binds these files as well as
JSON/JSONC. Referenced selectors cannot be removed or restored away; keeping them
permits token rotation. Unreadable or ambiguous resources cannot authorize removal,
and their original bytes remain untouched. Worker admission independently refuses
empty/substituted `$schema` values and incomplete project ancestry before native
startup, so it cannot trigger native schema repair or omit ancestor custom tools.
The stop must be confirmed by bounded Git checks using the Worker's filtered
HOME/XDG/PATH; `.git` existence alone is insufficient. Invalid markers, broken
worktree pointers and missing/rejecting Git fail before launch. The core Worker
journeys use real Git repositories/linked worktrees, place foreign configuration
and an import sentinel beyond the confirmed stop, and exercise false stops through
the public CLI before native database creation, tool import or model work.

## Ordinary routing metadata

The ordinary client automatically scans `plugins/hiroute-routing.js` under the
selected resource root. `OPENCODE_CONFIG` selects only a configuration file;
resources follow `OPENCODE_CONFIG_DIR`, or the OpenCode directory under
`XDG_CONFIG_HOME` when no extra root is selected. Do not add a second manual
`plugin` registration. Configure and Restore must preserve unrelated plugins.

OpenCode 1.19.3's native loader waits for resource-directory npm dependencies
before loading any external plugin, including this self-contained local file.
This happens before ACP initialization and before the hook's own deadlines can
apply. A cold resource directory can therefore block on native package installation
without sending a Gateway request. Validate dependency readiness in the final
HOME/XDG/resource scope with an actual managed initialization before model
acceptance. A downloaded package, an incomplete cache, or successful initialization
after removing the plugin does not establish readiness. Preserve cold-start
failures separately from acceptance with installed dependencies; increasing a
test timeout or warming a cache is not a fix for the loader's dependency boundary.

There is no verified 1.19.3 switch that preserves this plugin while bypassing that
wait. `OPENCODE_PURE` disables all external plugins and cannot substitute for
ordinary-client acceptance. Without the HiRoute plugin, native custom-provider
requests still supply session and parent-session headers, so Gateway retains
observed classification, execution capture and evidence-backed evaluation.
Purpose, current-event and compression references are weaker: auxiliary requests
sharing a session may be misattributed. This is a disclosed degraded capability,
not full metadata recovery. Never disable unrelated plugins or fabricate dependency
manifests to obtain a passing startup result.

### Recover from a blocked metadata-plugin startup

The public Agent settings selection accepts `routing_metadata: false` for
`opencode_additional`. It is saved with the model configuration and survives
daemon restarts and Desktop model edits. It omits only HiRoute's metadata plugin;
all selected Plans, protocols and Gateway routing remain available. Other external
plugins can still cause the native dependency wait.

For an existing managed installation, first return any native saved default that
references a managed selector to its original provider. Inspect the model status,
then use the normal model Restore transaction:

```sh
hiroute agents connect status CONTEXT_ID
hiroute agents restore preview --context CONTEXT_ID --facet model --apply-file restore-apply.json
hiroute agents restore apply --request-file restore-apply.json
```

Wait for the returned Operation to succeed (`hiroute operations get OPERATION_ID`)
and check status again. Restore preserves unrelated configuration and plugins.
If a pre-existing HiRoute plugin was borrowed, Restore leaves it in place: explicitly
review and disable that user-owned file before continuing. Preview reports
`native_routing_adapter_restore_required` or
`native_routing_adapter_external_resource` before changing model configuration;
neither error authorizes deleting another plugin.

Write a request file using the actual context and the complete intended Plan and
protocol selection. For example, `metadata-fallback.json` has this shape (replace
the two placeholder IDs and retain every Plan that should remain authorized):

```json
{
  "spec": {
    "schema_version": { "major": 2, "minor": 0 },
    "context_id": "CONTEXT_ID",
    "model": {
      "intent": "configure",
      "settings": {
        "mode": "opencode_additional",
        "allowed_plan_ids": ["PLAN_ID"],
        "plan_protocols": { "PLAN_ID": "responses" },
        "routing_metadata": false
      }
    },
    "collaboration": { "intent": "keep" },
    "management_skill": { "intent": "keep" }
  }
}
```

```sh
hiroute agents connect preview --request-file metadata-fallback.json --apply-file fallback-apply.json
hiroute agents connect apply --request-file fallback-apply.json
hiroute operations get OPERATION_ID
hiroute agents connect status CONTEXT_ID
```

Restart the ordinary OpenCode process in the same native profile. Classification,
capture and evaluation continue with the metadata limitation described above. This
two-step recovery briefly restores the original model configuration before applying
managed routes again; it does not silently remove an adapter from an active journal.
Once native dependencies are ready, preview and apply the same full selection with
`routing_metadata: true` (or omit the field), then restart OpenCode to load the
restored plugin. Re-enabling does not reconstruct any earlier missing evidence.

Acceptance must separately show a genuinely cold dependency directory completing
fresh A/B with actual classification, capture and exact prior-request assessment in
fallback mode; normal metadata mode must then recover actual request-header
adoption. Finish with full model Restore and actual default inference. Settings
transaction tests alone do not prove native startup or model execution.

The public `chat.headers` hook emits immutable v2 purpose/session/current-event
facts before optional enrichment. Known `title`, `summary` and `compaction` calls
skip the SDK. Ordinary calls use at most one 128-entry SDK page with a 50 ms abort
deadline and four unresolved reads per plugin instance. An SDK that ignores abort
retains its slot until settlement; saturation loses optional enrichment while
new ordinary calls remain routable. These are configured bounds, not measured
latency guarantees.

The exact current native message and chronological page positions locate a
completed successful summary and its exact compaction parent. Imported IDs are
not chronological, and transformed model history is not the SDK page. Missing,
reordered, cross-session or ambiguous entries omit the affected reference.
`turn_id` and `event_id` always identify this activation. Any old source belongs
only in optional compression references; Gateway still requires independently
captured execution boundaries before inheriting history.

The strict synthetic-only boolean `compaction_continue` shape is a private
OpenCode 1.19.3 capability. A confirmed continuation uses `input.kind=existing_turn`
so summary text cannot become new task input; its real output remains capturable.
A missing source may require temporary fallback for that call. Unsupported markers
and SDK failures retain observed-batch semantics. Callback IDs, copied overflow
parts, late child results, cold caches and new native sessions never prove human
origin. Actual child/fork session identity stays separate from parent/cache affinity.
Fresh same-session A/B must still classify, capture and evaluate actual A after
lookup failure, saturation, compaction and restart.

In OpenCode 1.19.3, failed native compaction can leave summary work pending. The
first ordinary input can trigger only the summary retry, leaving that input's main
task unexecuted. Preserve this limitation even when a later new A/B succeeds.
The verified recovery conditions are a new independent A/B after that retry, or
a successful explicit user ACP `/compact` retry followed by a new independent
A/B. Both require the additional summary request; summaries remain auxiliary and
must not acquire main-task capture. Conditional recovery does not establish
unconditional delivery of the first input.

OpenCode 1.19.3 background subagents require the explicit experimental opt-in
`OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS=true`. Without it, the native task gate
can refuse before child creation. Scope the flag to the selected isolated ACP
process and report flag-enabled acceptance separately from default support.

Cold continuation acceptance resumes the same owned session in a new native
process without `chat.message` prewarming. The exact OpenCode 1.19.3 installed
legacy JavaScript SDK forwards `auto: true` in `session.summarize` although its
legacy types omit that field. Bind this version-specific engine path to the exact
session/project and actually advertised provider/model. Require auxiliary-summary
exclusion, typed continuation, actual completed output capture and subsequent
independent A/B. Keep its evidence separate from ordinary CLI automatic pressure
and portable typed SDK support.

The Node hook regressions are component evidence. Native acceptance additionally
requires the actual installed loader, both transports, actual model-group changes,
exact assessment membership, tools/children, compaction/restarts, injected faults
and complete Restore followed by default inference. Controlled quality scores test
wiring and do not establish semantic scoring accuracy.

## Real product entry

Set `HIROUTE_PRODUCT_CANDIDATE_SHA` to the exact candidate and `HIROUTE_WORKER_OPENCODE_BINARY` to the selected official native binary. Unix acceptance also requires installed `rg` on the runner PATH: the fixture exposes that exact native tool in its private PATH so Skill loading does not depend on an upstream tool download. Run through the [configured validation runner](../../../docs/validation-routing.md):

```text
cargo test --locked -p hiroute-product-e2e --test opencode_delegation -- --ignored --nocapture --test-threads=1
```

Missing inputs fail. The tests build the production CLI and daemon and use their public Local Control/Worker commands. Only upstream model replies and fixture-owned user files are synthetic. A normal Cargo run that leaves these tests ignored does not establish acceptance.

The shared journeys cover saved Responses/Messages routes, daemon restart, grant rotation, default-reference refusal, independent model/Skill restore, passive source prepare/save with stale-credential refusal, actual installed Skill → public CLI → Worker, native Skill/read/edit/shell effects, concurrent routing/cancellation, missing-history refusal and exact Continue. OpenCode also exercises automatic native compaction and auxiliary requests on the frozen model route, including summary consumption after restart. A changed current Plan or current environment cannot select a new session on Continue.

The ordinary saved-model journey also checks installed
`plugins/hiroute-routing.js` auto-discovery through two native turns and actual
custom-decision history. It runs without Worker pure mode and binds both `cwd`
and `PWD` to the owned project. A missing-plugin control must still return the
model result and retain observed classification, actual capture and evaluation
with explicit uncertainty. Restoring the plugin cannot invent missing execution;
restarting Gateway cannot revive discarded process-local history. Fresh same-session
A/B after either boundary must recover actual capture and assessment. Model Restore
must remove the owned plugin and preserve the full baseline, followed by real native
startup and default inference without a provider/model override.

The explicit collaboration prerequisite check uses private configuration and history while loading the actual installed user Skill directory through OpenCode's resource loader. It disables native external `.agents`/`.claude` Skill discovery only for that isolated check, so an unrelated same-name global Skill cannot race the explicit resource. It validates exact path/body and a nonce-correlated trusted CLI result. It does not parse daily model configuration to authorize installation, inject a substitute installed Skill, or establish real delegated completion. Ordinary Agent and Worker Skill discovery remains native; a successful isolated check does not prove that an existing daily session loaded this file.

On Windows this private check explicitly selects system Windows PowerShell and
uses its literal native-command syntax; Qoder's Git Bash command contract is
unchanged. The nonce is emitted only after a successful CLI exit. Static-source
acceptance also includes independent provider credential failures (symbolic/hard
links or oversized files): these stay non-importable while the valid source completes
public Scan/Prepare/Save and still rejects stale credentials.

## Evidence and platform limits

Require every selected business case to be green in its scenario report. A successful build, fixture response or process exit alone is insufficient. Retain the exact source SHA, native package identity, selected protocol, task/native-session identities, source request kinds, owned-child cancellation and preserved native files. The product tests must not promote SDK metadata or mock IPC to native success.

macOS/Linux/native Windows each need their real filesystem, installation and process boundary evidence. The Unix product entry above does not establish Windows acceptance. Windows compilation and native execution follow the existing [Windows validation workflow](../../../docs/validation-routing.md#windows-iteration-and-release-validation); neither another OS nor a cross-compile replaces execution. Browser component checks cover OpenCode forms separately and do not prove native Desktop authorization or persistence.

For the native Windows boundary, configure the pinned `HIROUTE_WORKER_OPENCODE_BINARY`
path in the existing HOME Windows validation configuration and include its native
package metadata. Compile the `windows_headless` target through the managed MSVC
workbench, export/stage its verified bundle, and select
`actual_windows_opencode_worker_native_history_and_cancellation` with the existing
`validation.py windows run` entry. It reuses the public source/Plan/Worker fixture,
real Skill and file-tool receipts, Job cancellation checks and daemon restart with
exact Continue. Native roots are created before daemon startup and borrowed
configuration must remain byte-identical. This case establishes those Windows
boundaries; saved ordinary model routes, static import and native Main delegation
retain their separate journeys above.
