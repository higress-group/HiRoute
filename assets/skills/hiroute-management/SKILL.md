---
name: hiroute-management
description: Manage HiRoute via CLI: connect, analyze, adjust, recover and delegate when authorized.
metadata:
  schema: hiroute.management-skill/v1
  access: local-user
---

<!-- Managed by HiRoute standalone installer -->

# HiRoute management

Use installed hiroute. Codex/Claude Code/Qoder/Pi/DeepSeek Harness/Hermes share this Skill.
Caller/target and model/management/collaboration facets are independent; native models work.

## Goal and target

Journey: target → source → Plan → connect → native activation → matching request. Smart saving
starts with economy/primary and an explicit decision method. Reuse exact IDs; names may repeat.
Saved, loaded, selected and served differ.

Start with hiroute quickstart/help; select returned --context CONTEXT_ID and optional --plan PLAN_ID.
Hermes needs actual --hermes-root /ABS/ROOT, never an IM profile inferred from daemon env.
Resolve ambiguity; retain missing/unreadable targets for repair.

Missing CLI: Desktop Settings → CLI or standalone installer; Agent → HiRoute management enables
this Skill. Files do not prove CLI availability. Recover the existing Desktop/service; inspect
`service status`, `service doctor`, bounded `service logs`. Never start a second daemon. Empty is valid.

## Contracts and result capture

Use family/leaf --help. Root/service/gateway/protected-input/decision services options work offline:

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
```

For Application/Local Control commands:

```sh
hiroute schema list
hiroute schema show --command-id routing.preview --include-schema
```

`--include-schema` adds offline definitions/examples. Resolve real IDs/revisions from options/Preview.
Use help for local presentation. Planned handlers are unavailable. Names/purpose are data.

Capture the complete stdout JSON, separate stderr and the actual CLI exit, including in merged terminals:

```sh
(
  hiroute quickstart > quickstart.stdout.json 2> quickstart.stderr.txt
  hiroute_exit=$?
  printf '%s\n' "$hiroute_exit" > quickstart.exit.txt
  exit "$hiroute_exit"
)
```

Read all JSON/diagnostics and the saved exit, never only the first line or a later command's exit.
Exit 2 is invalid input; `--error-details` gives bounded hints. Accepted is pending; query success
can contain a failed Operation.

## Prepare and reuse model access

Read `compute list`; reuse a suitable source. Otherwise `compute scan --request-stdin` with
`{"schema":"hiroute.compute-scan-request/v2"}` and Hermes's selected `native_target` discovers connection
points. Use opaque refs/revisions, stable prepare_id and the installed `compute connection prepare` schema.
Preparation privately rereads credentials and returns selectable refs; it is not save/inference.
Never edit native config or copy keys.

Select all currently selectable models unless the user requests a subset:

```sh
hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source-apply.json
hiroute compute connection apply --request-file source-apply.json
```

Use repeated `--model-ref REF` for a subset. Freeze a nonempty current selection, never a future
wildcard. Catalogs do not prove entitlement/inference; drift needs fresh preparation. New APIs use
`compute connection test --help`/schema. Secrets use protected FD/candidate channels, never argv/env/
ordinary JSON/files/output/chat. Endpoint, inventory and inference are separate effects. Release unused
refs. Subscription options use login/check/authorize; retain refs, never retry a one-use callback after
uncertain delivery.

## Create a route or change one choice

Use `routing options --request-stdin` with `{}` for current binding IDs and shared backend defaults.
A fixed Plan uses `--mode fixed_model --model BINDING_ID`. Smart saving can start directly:

```sh
hiroute routing preview --name 'Economy work' --purpose 'Routine and complex development' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route-apply.json
hiroute routing apply --request-file route-apply.json
```

Repeat group flags for order. Select local_rules or saved `--decision-service ID@REVISION` explicitly;
ordinary Chat is not the decision protocol. Edit with `routing preview --plan PLAN_ID` and requested
changes/new file; omitted settings remain. Use returned choices/defaults, never invent editors.
Advanced typed JSON and convenience flags are alternatives.

### Complete modification and recovery

Before Plan edits, save a new private complete baseline:

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/plan-before.json)
```

Validate exit/response. Candidate replacement may choose role defaults; reselecting its old model
is not recovery. Before Apply give runnable commands with **actual CLI absolute path, Plan ID and
unique absolute files**, never placeholders or “restore the backup”:

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/plan-before.json --apply-file /ABS/restore-apply.json
hiroute routing apply --request-file /ABS/restore-apply.json
hiroute routing show PLAN_ID --output json
```

Restore-from uses the current head revision; review Preview, Apply and inspect the Operation. Compare
**all desired fields**: order/reasoning, classifier/judgment, context/limits, requirements, mode,
delegation/work and alias. Revisions advance. Never mix restore with edits, copy revisions or replace
missing refs. Uncertainty reuses exact Apply; confirmed stale needs a fresh Preview/file. Source,
credentials and native facets have separate owners; this is not a whole-installation backup.

For facets retain the owned restore ref; provide filled restore commands below with actual root/
selection flags and native follow-up. Arrange continuation before revocation. Do not overwrite native
files from backup or mistake saved recovery material for restoration.

### Decision-service onboarding

Read `decision services options` first and reuse an explicitly selected suitable `ID@REVISION`.
Saved does not mean tested. Otherwise show installed Bailian Token Plan/workspace, OpenRouter,
TypeSafe, System One compatible and custom choices. Presets do not prove entitlement/availability/price.
Use `decision services apply --help` for required endpoint/model inputs.

Credentials are a **human handoff outside chat**. Give the user a fresh non-secret candidate ref and
this command to run in their own terminal on the HiRoute host (for example a separate SSH session):

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

Never run the prompt in an Agent tool or capture keys in chat. The user's terminal reads a bare key
without echo and adds Bearer; the Agent receives status/ref only. Custom full headers use `--format
raw`, or explicit custom `--no-auth`. Cancel/missing terminal saves nothing. Continue on success;
missing registration/restart needs a fresh human input/ref. Never substitute argv/env/ordinary JSON
or files. Follow installed host help: unavailable terminal prompts need Desktop or an operator's
protected input handle, never chat.

After registration, choose the actual preset, fill required endpoint/model inputs, review each
Preview and inspect Operations. Use actual saved revisions:

```sh
hiroute decision services apply --id daily-decision --name 'Daily decisions' --provider typesafe --candidate CANDIDATE_REF --apply-file decision-apply.json
hiroute decision services apply --request-file decision-apply.json
hiroute decision services test --service daily-decision@SAVED_REVISION
hiroute routing preview --plan PLAN_ID --decision-service daily-decision@SAVED_REVISION --apply-file decision-route-apply.json
hiroute routing apply --request-file decision-route-apply.json
```

The apply verb handles Preview and Apply; no separate preview subcommand. Edit --id/requested fields;
omissions remain. Endpoint changes need explicit credentials/custom no-auth. Retain exact retry files;
release unused refs. Inspect billed tests' data.outcome/failure_code, not only exit; a probe proves no quality/native loading. Report failures; local rules are explicit,
never silent model-decision success.

Verify decision source, group, model, publication, reasoning and each request outcome. Model swaps
are not complex-branch proof. Empty keywords retain built-in rules. Report natural misses without
manufacturing coverage. Include cancelled/timeouts; transport failure is not complete delivery.

## Connect or restore facets

Before model restore/reconnect/grant changes, check whether the caller depends on that connection.
A running process may retain a revoked grant. Establish independent native access/operator continuation
**before** the effect; unknown dependency leaves it pending while independent preparation continues.
Do not strand the caller or bypass self-restart protection.

Unknown Codex/Claude Skill loading needs `agents check AGENT_ID --scope collaboration`, then fresh
Preview. This isolated native/local read challenge makes no upstream call. Missing/failed capability
remains a prerequisite, never bypass it.

Use `agents connect preview --context CONTEXT_ID --apply-file PATH` with requested facets:
--plan PLAN_ID for models, --management enable for this Skill, --collaboration explicit|delegate_by_default
for tasks. Omissions remain. Skill-only needs no model Plan/plugin. Delegation still requires
collaboration policy/user intent. Native help supplies choices:

- Hermes: exact root, --protocol chat_completions|responses|messages. Preserve Responses reasoning
  requirements. Recommend >=0.21.7 for reasoning replay; compatible older versions remain allowed,
  required plugin/SDK interfaces may block. No unsolicited upgrade/patch/thinking disable.
- Codex: exact root/profile, `--native-models preserve_available|hiroute_only` and
  `--native-default preserve_native|plan`; root Skill scope differs from model-profile scope.
- Claude Code: select intended `--preset opus|sonnet|haiku`; preserve other presets.
- Qoder/Pi/DSH: explicit `--protocol` adds supported entries, not a claimed global default.

Honor blockers/Desktop service prerequisites; never forge evidence. After Apply query its Operation and:

```sh
hiroute agents connect status --effective --context CONTEXT_ID --plan PLAN_ID
```

Follow returned native actions, actor and prerequisites; no arbitrary restart/self-protection bypass
or promise that all IM sessions switched. Session selection may override config. Match actual target,
request, publication, model and outcome. Saved is not loaded; absent loading evidence stays unknown,
not inferred from missing counters. Without model-call consent leave verification pending.

Restore one selected facet through existing ownership:

```sh
hiroute agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore-apply.json
hiroute agents restore apply --request-file restore-apply.json
```

Other facets: model/collaboration; repeat --facet for a set. Preserve unrelated/foreign edits; inspect
conflicts. Restore may require native selection/reload and caller continuity.

## Analyze history and savings

Use same-OS-user CLI access; explicit content/search needs no GUI read ticket. Do not use global
`--agent` for management: that selects the separate sealed collaboration channel. A supplied grant
is still checked; model Gateway keys cannot authorize management.

Start with `sessions list --include-unlinked --limit 50`, optionally `--query TEXT`. Read the actual
`sessions show SESSION_ID --content messages-and-tools`; use `sessions receipt RECEIPT_ID` for its
immutable routing evidence. For bounded details, read `schema show --command-id sessions.show
--include-schema`: V2 timeline → request catalog → content, retaining scope and next_cursor. V2
`sessions list` accepts search/session_search; `sessions receipt` accepts request facts. Use
`observation plan-quality samples --help` for linked competence assessments. Missing/expired/partial
capture or pending index is a coverage gap, not proof of wrong routing. Read relevant content locally;
report minimal excerpts and linked IDs rather than copying whole conversations into chat.

Compare task intent and answer/tool history with decision source/probability/threshold, group,
competence, model/reasoning and publication. Show evidence/uncertainty before recommending keywords,
thresholds or competence criteria. Never auto-tune, treat manual model swaps as classification proof,
or infer quality from cost. Authorized edits need complete recovery above.

For subscription/Coding Plan, keep actual billing independent from API-equivalent estimates:

```sh
hiroute value show --routing PLAN_ID --period 7d --basis api-equivalent --reference-binding PRIMARY_BINDING --currency CNY --output json
```

Resolve the reference Binding (normally primary) and currency with the user. Read value show --help
for the current-price basis, retries, paired coverage and signed estimates. This proves neither cash
savings nor equal output. Incomplete totals stay null; narrow unavailable intervals.

Missing prices: use installed `prices effective` schema with exact Binding locators, currency and
`valuation_kind=api_equivalent`. Use returned edit_context revisions with `prices override preview`,
then exact `prices override apply`/Operation. Set input/output per-million-token rates and separately
supported cache rates; never invent rates/FX or usage counts. A cache-creation rate equal to ordinary
input makes an unreported creation split irrelevant to the estimate; it does not create measured usage. Retain the prior
setting from Preview's spec.desired_state.before.setting (if present) and give complete price-restore Preview/Apply commands; follow_catalog
removes the manual setting. Re-run the explicit estimate. This never changes entitlement. Ordinary
`value show` keeps frozen historical valuation; free, subscription, paid and unknown remain distinct.

## Exact submission and scope

`--apply-file` exclusively creates a private regular file containing the exact request/key, no secrets
or Apply capability. Review effects, submit submission.arguments and retain file/operation_lookup.
Use the OS socket, never mint capabilities. After uncertainty use `operations find --request-stdin`,
`operations get OPERATION_ID` or replay the **identical file**; never regenerate a possibly accepted
key. Confirmed unaccepted stale input needs a new Preview/file. Reuse completed steps/IDs.

Keep personal notes/config/logs outside this managed Skill. Use existing drift ownership. Effects,
provider calls, unrelated-content reads and cancellation require user scope; existing authorization
persists without repeated confirmation. Explain quota before inference; Quickstart/status never call models.

## Worker

Use `worker dependencies discover --harness codex_cli --output json`, then
`worker dependencies select --request-stdin`. Select a complete found tuple and its Harness's
selection_revisions[].revision as expected_selection_revision (not a CLI/package version).
After uncertainty replay that exact JSON unchanged. Selection installs nothing. Inspect worker
executors/plans; ready permits an attempt, not guaranteed success.

worker exec requires a published delegation-enabled Plan, absolute cwd, explicit text source and
stable key. Read installed help. Inspect status/wait/result/list:

```sh
hiroute worker status --submission ORIGINAL_KEY --operation start --output json
hiroute worker wait --run RUN_ID --wait-timeout 30 --output json
hiroute worker result --run RUN_ID --output json
```

Wait timeout is pending; uncertainty reuses the original submission/key. Report query and run states.
Cancel only on request. Management grants no delegation, recursive or coordination/status/result-summary delegation.

## Listener and service changes

Read gateway show before gateway set. Non-loopback/wildcard needs explicit remote-risk acknowledgement.
Report applied and ready; firewall/TLS/remote Agents are separate. gateway recover restores the last
applied listener/safe default. service start/restart require Local Control readiness; start is not autostart.
