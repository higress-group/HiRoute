---
name: hiroute-management
description: Manage HiRoute via CLI: connect, analyze, adjust, recover and delegate when authorized.
metadata:
  schema: hiroute.management-skill/v1
  access: local-user
---

<!-- Managed by HiRoute standalone installer -->

# HiRoute management

Use installed hiroute. Agents share this Skill; caller/target and model/management/collaboration are independent.

## Goal and target

Journey: target → source → Plan → connect → native activation → matching request. Reuse exact IDs;
names may repeat. Saved, loaded, selected and served differ.

Start with hiroute quickstart/help; select returned --context CONTEXT_ID and optional --plan PLAN_ID.
Hermes needs --hermes-root /ABS/ROOT, not daemon env. Resolve ambiguous/unreadable targets.

Missing CLI: Desktop Settings → CLI or standalone installer. Agent → HiRoute management enables
this Skill. Recover via status/doctor/bounded logs; never start a second daemon.

## Contracts and result capture

Use family/leaf --help and schema show --command-id COMMAND --include-schema. Planned handlers are
unavailable; get IDs/revisions from options/Preview.

Save full stdout JSON, stderr and actual exit privately. Exit 2 means invalid input;
--error-details gives hints. Accepted is pending; query success can contain a failed Operation.

## Prepare and reuse model access

Reuse compute list sources; otherwise compute scan --request-stdin with
`{"schema":"hiroute.compute-scan-request/v2"}` and selected Hermes native_target discovers endpoints.
Use returned refs/revisions, stable prepare_id and installed prepare schema. Preparation rereads
credentials privately; selectable refs prove no save/inference. Never edit native config or copy keys.

Select all currently selectable models unless the user requests a subset:

```sh
hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source-apply.json
hiroute compute connection apply --request-file source-apply.json
```

Use --model-ref for a subset; freeze a nonempty selection, never a future wildcard. Catalogs prove no
entitlement/inference; drift needs fresh preparation. New APIs use connection test help/schema.
Secrets use protected FD/candidates, never argv/env/ordinary JSON/files/output/chat. Endpoint/inventory/
inference are separate. Release unused refs. Subscription login/check/authorize retains refs; never
replay a one-use callback after uncertain delivery.

Native defaults to Direct. `compute network show/resolve`: policy/consumers/routes, no I/O.
`compute network preview/apply`: current source/network revisions and original retry key; repair needs
no remote pass. Changes affect new requests; admitted work keeps its route. Inherit uses startup env
(restart to change). Manual HTTP(S) URLs lack credentials; bypass uses the API URL. Keep subscription
settings separate. `compute connection test-network`: V2, draft revision 0; discovery is non-probing.
`decision network test`: saved ID/revision/config digest, shared policy at its decision URL.
Tests may bill: inspect `check`/`network`/typed `failure`. Route changes stale success.
Proxy auth never justifies provider-Key replacement/disablement.

## Create a route or change one choice

routing options --request-stdin with `{}` supplies IDs/defaults. Fixed uses --mode fixed_model
--model BINDING_ID. Smart saving:

```sh
hiroute routing preview --name 'Economy work' --purpose 'Routine and complex development' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route-apply.json
hiroute routing apply --request-file route-apply.json
```

Repeat group flags for order. Select local_rules or saved --decision-service ID@REVISION;
Chat is not the decision protocol. Edit with --plan PLAN_ID and changes/new file; omissions remain.
Use returned choices/defaults; typed JSON and flags are alternatives.

### Plan recovery

Before Plan edits, save a new private complete baseline:

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/plan-before.json)
```

Validate exit/response. Replacement may choose role defaults; reselecting the old model is not recovery.
Before Apply give runnable commands with **actual CLI absolute path, Plan ID and unique absolute files**:

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/plan-before.json --apply-file /ABS/restore-apply.json
hiroute routing apply --request-file /ABS/restore-apply.json
hiroute routing show PLAN_ID --output json
```

Review restore Preview/Operation and compare **all desired fields**: order/reasoning, classifier/
judgment, context/limits, requirements, mode, delegation/work and alias. Restore uses the current head;
revisions advance. Never mix edits with restore, copy revisions or replace missing refs. Reuse exact
Apply after uncertainty; confirmed stale needs fresh Preview. Source/credentials/facets have separate owners.

For facets retain the owned restore ref; fill restore commands below with root/selection flags and
native follow-up. Arrange continuation before revocation. Never overwrite native files from backup;
saved recovery material is not restoration.

### Decision-service onboarding

Read decision services options; reuse a selected ID@REVISION or show installed provider/custom choices.
Saved/presets prove no testing, entitlement, availability or price.

Credentials require a **human handoff outside chat**. Give a fresh non-secret candidate ref and this
command for the user's terminal on the HiRoute host (for example SSH):

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

Never run the prompt in an Agent tool. The human terminal reads a bare key without echo and adds
Bearer; the Agent gets status/ref only. Custom full headers use --format raw or explicit --no-auth.
Cancel/missing terminal saves nothing; restart/missing registration needs fresh human input/ref.
Never substitute chat/argv/env/ordinary JSON/files. Without a terminal use Desktop/protected handles.

After registration, use `decision services apply --help` to fill the actual provider/endpoint/model
and registered candidate. First prepare --apply-file, review it, then apply --request-file. Inspect
Operations, run `decision services test --service ID@SAVED_REVISION`, and explicitly select that
saved revision with `routing preview --plan PLAN_ID --decision-service ID@REVISION --apply-file PATH`
and `routing apply --request-file PATH`.

The apply verb handles Preview/Apply; edit --id/requested fields, preserving omissions. Endpoint
changes need explicit credentials/no-auth. Keep retry files, release unused refs, inspect billed tests
data.outcome/failure_code. A probe proves no quality/native loading. Local rules require explicit choice.

Verify decision source, group, model, publication, reasoning and each request outcome. Model swaps
are not complex-branch proof. Empty keywords retain built-in rules. Report natural misses without
manufacturing coverage. Include cancelled/timeouts; transport failure is not complete delivery.

## Connect or restore facets

Before model restore/reconnect/grant changes, check caller dependency; running processes can retain
a revoked grant. Establish independent native/operator continuation **before** effects. If unknown,
leave the effect pending and continue preparation. Never strand the caller or bypass self-restart protection.

Unknown Codex/Claude Skill loading needs `agents check AGENT_ID --scope collaboration`, then fresh
Preview. This isolated native/local read challenge makes no upstream call. Missing/failed capability
remains a prerequisite, never bypass it.

Use `agents connect preview --context CONTEXT_ID --apply-file PATH` with requested facets:
--plan PLAN_ID for models, --management enable for this Skill, --collaboration explicit|delegate_by_default
for tasks. Omissions remain. Skill-only needs no model Plan/plugin. Delegation still requires
collaboration policy/user intent. Native help supplies choices:

Binary/Skill upgrades leave model plugins unchanged. Preview/Apply the saved model facet with
access_token=keep, then reload/start native; preserve other facets. Use native help options.
Hermes needs exact root/protocol and preserved Responses reasoning. Recommend >=0.21.7 for replay;
compatible older versions remain allowed, subject to required plugin/SDK interfaces. No unsolicited
upgrade/patch/thinking disable. Codex needs exact root/profile and native/default choice; preserve
other Claude presets. Qoder/Pi/DSH entries claim no global default.

Honor blockers/Desktop service prerequisites; never forge evidence. After Apply query its Operation and:

```sh
hiroute agents connect status --effective --context CONTEXT_ID --plan PLAN_ID
```

Follow native actions/prerequisites; no arbitrary restart/self-protection bypass or promise that all
IM sessions switched. Session selection may override config. Match target/request/publication/model/
outcome. Absent loading evidence stays unknown. Without model-call consent verification stays pending.

Restore one selected facet through existing ownership:

```sh
hiroute agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore-apply.json
hiroute agents restore apply --request-file restore-apply.json
```

Other facets: model/collaboration; repeat --facet for a set. Preserve unrelated/foreign edits; inspect
conflicts. Restore may require native selection/reload and caller continuity.

## Analyze history and savings

Same-OS-user content/search needs no GUI ticket. Global `--agent` selects sealed collaboration,
not management. Supplied grants are checked; model keys grant no management authority.

Choose the user's goal: switch explanation, conversation, expense ranking or savings. Start one scope:

```sh
hiroute value show --view summary --period 7d
hiroute value show --group-by session --order-by cost-desc --period 7d
hiroute sessions list --view switches --period 7d
hiroute sessions list --view conversations --period 7d --query TEXT
```

Use sessions/value help. Report requested/effective bounds, offsets and clipping. Days are
service-local; never mix time modes. New views reject --model-switch.

Retain analysis_scope across related/narrower reads; add no time selector. Start search with
--query TEXT; keywords cannot change within a scope. Keep old scopes for comparison. Page with
unchanged view/filters/limit to END; page length/member preview is not a total. Commands:

```sh
hiroute sessions show SESSION_ID --view routing --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --analysis-scope TOKEN
hiroute value show --group-by request --order-by cost-desc --session SESSION_ID --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --request REQUEST_ID --analysis-scope TOKEN
```

Keep scope across requests/compaction/follow-ups. Traffic/settlement/prices cannot change pinned
results. Never silently refresh or compare cross-scope totals. Expiry, withdrawn proof, affected
deletion or authority change needs a fresh initial query; give its filled command.

Explain decisions and served model/reasoning/publication. Stages are not switches. between_turn needs
proven adjacent completed user turns. continuation_boundary is compression, possibly mid-turn;
turn_context_reset/pending/unknown prove no adjacency. Missing lineage separates members; matching
text/time proves no link. Key relay/fallback differ from turn changes. Count each request/attempt once.

For reference_only/partial use returned content_id/request_id and sessions.show schema for V2 pages.
Keep cursor/scope and minimal linked excerpts; never expose private provider state. sessions receipt
opens immutable evidence; plan-quality samples explain stages. Legacy list --include-unlinked --query
and show --content messages-and-tools remain. Use public reads, never repository/manual joins.

Ranking needs no reference. Choose one currency/recorded valuation kind, or --basis api-equivalent
for current prices. Summary defaults to that estimate and an unambiguous primary model; resolve
reference/currency choices with real binding IDs. Never mix historical/current prices, currencies or
valuation kinds. Unknown is not free/zero. Report highest-known-only rankings, signed paired savings
and missing/excluded coverage; zero denominator has no percentage. Decision overhead is unmeasured.
Coding Plan estimates are API-equivalent, not cash bills or quality proof. Ordinary value show keeps history.

For pending/index_unavailable report coverage/progress/stall. For timeout/reader_busy follow
error.observation_read: retry the valid scope after retry_after_ms, bound retries, then report the
exact failure, not an absent subsystem. Narrow to a request/smaller page. Missing/deleted/expired
capture proves no routing error.
Compare task/answer/tools with decision source/group/probability/threshold/competence/reasoning before
proposing changes. No auto-tuning, quality inference from cost or manual-swap classification proof.
Explain uncertainty; authorized edits need a fresh complete baseline and filled recovery commands.

Missing prices: prices effective schema needs exact Binding/currency/valuation_kind=api_equivalent.
Use returned edit_context in prices override preview, then exact apply/Operation. Set per-million-token
input/output/supported cache rates; invent no prices/FX/usage. Equal cache-creation/input rates remove
that estimate sensitivity, not its measurement gap. Keep complete before.setting; give filled restore
Preview/Apply commands (follow_catalog removes override). Re-run estimates without changing entitlement;
free/subscription/paid/unknown stay distinct.

## Exact submission and scope

--apply-file creates a new private exact request/key file without secrets/capabilities. Review effects;
submit submission.arguments; retain file/operation_lookup. Use the OS socket; never mint capabilities.
After uncertainty use operations find/get or replay the **identical file/key**. Confirmed unaccepted
stale input needs fresh Preview. Reuse completed IDs.

Keep notes outside this Skill. Honor ownership/user scope for effects, models, unrelated
content and cancellation. Authorization persists. Explain quota. Quickstart/status never call models.

## Worker

Use worker help. `worker dependencies discover --harness codex_cli`, then
`worker dependencies select --request-stdin` select a found complete tuple using Harness
selection_revisions[].revision as expected_selection_revision, not a CLI/package version.
Selection installs nothing. Read executors/plans. worker exec needs a published delegation-enabled
Plan, absolute cwd, explicit text source and stable key. Status/wait/result/list retain submission/run
ID; wait timeout is pending. Replay the identical request/key after uncertainty. Cancel only on request.
Management grants no recursive or coordination/status/result-summary delegation; readiness guarantees no success.

## Listener and service changes

Read gateway show before set; remote/wildcard needs explicit risk acknowledgement. Report applied/
ready separately from firewall/TLS. gateway recover restores the applied/safe listener. Check Local
Control readiness after service start/restart; start is not autostart.
