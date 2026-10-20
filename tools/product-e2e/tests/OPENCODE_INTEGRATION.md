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

## Real product entry

Set `HIROUTE_PRODUCT_CANDIDATE_SHA` to the exact candidate and `HIROUTE_WORKER_OPENCODE_BINARY` to the selected official native binary. Unix acceptance also requires installed `rg` on the runner PATH: the fixture exposes that exact native tool in its private PATH so Skill loading does not depend on an upstream tool download. Run through the [configured validation runner](../../../docs/validation-routing.md):

```text
cargo test --locked -p hiroute-product-e2e --test opencode_delegation -- --ignored --nocapture --test-threads=1
```

Missing inputs fail. The tests build the production CLI and daemon and use their public Local Control/Worker commands. Only upstream model replies and fixture-owned user files are synthetic. A normal Cargo run that leaves these tests ignored does not establish acceptance.

The shared journeys cover saved Responses/Messages routes, daemon restart, grant rotation, default-reference refusal, independent model/Skill restore, passive source prepare/save with stale-credential refusal, actual installed Skill → public CLI → Worker, native Skill/read/edit/shell effects, concurrent routing/cancellation, missing-history refusal and exact Continue. OpenCode also exercises automatic native compaction and auxiliary requests on the frozen model route, including summary consumption after restart. A changed current Plan or current environment cannot select a new session on Continue.

The explicit collaboration prerequisite check uses private configuration and history while loading the actual installed user Skill directory through OpenCode's resource loader. It disables native external `.agents`/`.claude` Skill discovery only for that isolated check, so an unrelated same-name global Skill cannot race the explicit resource. It validates exact path/body and a nonce-correlated trusted CLI result. It does not parse daily model configuration to authorize installation, inject a substitute installed Skill, or establish real delegated completion. Ordinary Agent and Worker Skill discovery remains native; a successful isolated check does not prove that an existing daily session loaded this file.

On Windows this private check explicitly selects system Windows PowerShell and
uses its literal native-command syntax; Qoder's Git Bash command contract is
unchanged. The nonce is emitted only after a successful CLI exit. Static-source
acceptance also includes independent provider credential failures (linked, exposed
or oversized files): these stay non-importable while the valid source completes
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
