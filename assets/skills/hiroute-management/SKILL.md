---
name: hiroute-management
description: Manage HiRoute through its installed CLI for connections, model competence, routing suggestions, authorized follow-up, recovery and delegation.
metadata:
  schema: hiroute.management-skill/v1
  access: local-user
---

<!-- Managed by HiRoute standalone installer -->

# HiRoute management

One Skill serves all supported Agents. Distinguish the caller, managed target, and independent
model/management/collaboration facets. Use installed public commands, not repository access.

## Target and contracts

Use quickstart/help and returned --context CONTEXT_ID/--plan PLAN_ID; resolve ambiguous targets/IDs.
Hermes needs --hermes-root /ABS/ROOT. Saved, loaded, selected and actually served are separate facts.

Missing CLI: Desktop Settings → CLI or standalone installer. Agent → HiRoute management enables this
Skill independently of models/delegation. Recover via status/doctor/bounded logs; keep one daemon.

Use family/leaf --help and schema show --command-id COMMAND --include-schema. For reads without full
schemas, use the installed descriptor/leaf help. Planned handlers are unavailable; resolve IDs,
revisions and choices from current results/defaults. Capture complete stdout JSON, separate stderr
and actual exit privately. Exit 2 is invalid input; --error-details gives hints. Accepted is pending;
query success can contain a failed Operation. Never parse only the first output line.

## Model access

Reuse compute list sources; otherwise compute scan --request-stdin with
`{"schema":"hiroute.compute-scan-request/v2"}` and selected Hermes native_target discovers endpoints.
Use refs/revisions, stable prepare_id and installed prepare schema; preparation rereads credentials
privately. Refs/catalogs prove no save/entitlement/inference. Never patch native config.

Select all currently selectable models unless the user requests a subset:

```sh
hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source-apply.json
hiroute compute connection apply --request-file source-apply.json
```

--model-ref selects a subset; freeze a nonempty selection, not a future wildcard. Drift needs fresh
preparation. Secrets use protected FD/candidates or the human terminal, never argv/env/ordinary
JSON/files/chat. Release unused refs. Never replay a one-use subscription callback after uncertainty.

Native networking defaults to Direct. compute network show/resolve is non-probing; preview/apply
uses current revisions/exact retry key. Changes affect new requests; Inherit uses startup env.
Subscription settings are separate. Read connection test-network/decision network test help for
input identity. Tests may bill; inspect check/network/failure. Route changes stale success; proxy
auth does not justify provider-Key replacement/disablement.

## Create or edit routing

routing options --request-stdin with `{}` gives IDs/defaults. Fixed uses --mode fixed_model
--model BINDING_ID. Difficulty routing keeps the smart_saving CLI identifier:

```sh
hiroute routing preview --name 'Difficulty routing' --purpose 'Routine and complex work' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route-apply.json
hiroute routing apply --request-file route-apply.json
```

Repeat group flags for order. Choose local_rules or saved --decision-service ID@REVISION explicitly;
Chat is not the decision protocol. Edit with --plan PLAN_ID and requested fields/new file; omissions
remain. Use returned choices.

### Complete Plan recovery

Before edits, save a new private complete baseline and validate its exit/response:

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/plan-before.json)
```

Before Apply fill these recovery commands with **actual CLI absolute path, Plan ID and unique absolute
files**; selecting old model names alone can restore different defaults:

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/plan-before.json --apply-file /ABS/restore-apply.json
hiroute routing apply --request-file /ABS/restore-apply.json
hiroute routing show PLAN_ID --output json
```

Compare all desired fields: order/reasoning, classifier/judgment, context/limits, requirements, mode,
delegation/work and alias. Inspect Preview/Operation. Restore uses the current head; revisions advance.
Do not mix restore with edits, copy revisions or substitute missing refs. Later restoration after
completed recovery needs a fresh Preview/Apply identity. Sources/credentials/Agent facets recover separately.

### Decision-service onboarding

Use decision services options: reuse the explicitly chosen ID@REVISION or show installed provider/
custom choices. Saved presets prove no testing/availability. Credentials require a human handoff
**outside chat**: give a fresh non-secret candidate ref and this command for the user's own terminal:

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

Never run the prompt in an Agent tool. It reads a bare key without echo and adds Bearer; receive only
status/ref. Full headers use --format raw; no authentication requires --no-auth. Cancel/missing terminal
saves nothing; restart needs fresh input/ref. Desktop/protected handles are the terminal alternative.

Use decision services apply help to fill provider/endpoint/model and registered candidate. Prepare
--apply-file, review, apply --request-file, inspect Operation and run decision services test
--service ID@SAVED_REVISION. Bind that revision via routing preview --plan PLAN_ID
--decision-service ID@REVISION and ordinary Apply. Edits use --id/requested fields, preserving omissions;
endpoint changes require explicit credentials/no-auth. Keep retry files/release unused refs. Inspect
billed test data.outcome/failure_code; a probe proves no quality/native loading.

Verify classifier/group/model/publication/reasoning and every matching request outcome. Model swaps
prove no complex-branch coverage. Empty keywords retain built-in rules. Report natural misses and
cancelled/timeouts; transport failure is not completed delivery.

## Connect or restore Agent facets

Before restore/reconnect/grant changes, establish independent continuation if caller access may be
revoked; running processes can retain old grants. Unknown dependency leaves the effect pending for
an operator. Keep self-restart protection.

Unknown Codex/Claude Skill loading needs agents check AGENT_ID --scope collaboration, then fresh
Preview. This isolated native/local challenge makes no upstream call; failed prerequisites remain.

agents connect preview --context CONTEXT_ID --apply-file PATH takes requested facets: --plan PLAN_ID,
--management enable, --collaboration explicit|delegate_by_default. Omissions remain; management-only
needs no Plan/plugin. Delegation requires user intent/collaboration policy.

Binary/Skill upgrades leave model plugins unchanged. Refresh the saved model facet with access_token=keep,
then reload/start native as required; preserve other facets. Follow native help: Hermes requires exact
root/protocol and preserved Responses reasoning; recommend >=0.21.7 for replay, while compatible older
versions remain allowed subject to plugin/SDK interfaces. No unsolicited upgrade/patch/thinking disable.
Codex needs exact root/profile/native model choice; preserve other Claude presets. Qoder/Pi/DSH entries
claim no global default.

Honor blockers/native prerequisites; inspect Apply Operation and:

```sh
hiroute agents connect status --effective --context CONTEXT_ID --plan PLAN_ID
```

Session selection may override config; one change proves no global IM switch. Match target/request/
publication/model/outcome; absent loading/model-call consent leaves verification pending.

Restore selected owned facets using the retained ref, preserving foreign edits and caller continuity:

```sh
hiroute agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore-apply.json
hiroute agents restore apply --request-file restore-apply.json
```

Repeat --facet for model/collaboration or a set. Give filled root/selection flags and native follow-up;
inspect conflicts. Backup files prove neither restoration nor native reload.

## Review competence and routing suggestions

Resolve authorized Plan/context IDs:

```sh
hiroute observation plan-trust suggestions --output json
hiroute observation plan-trust --plan-id PLAN_ID --revision active --group-by model --output json
```

Filter the workspace feed to selected Plans. The report explains keep/missing evidence. Show revision,
branch/model/reasoning, interval/cohort, scored/executed/session counts, competence/coverage/protections.
Keep partial/unscored/awaiting/unavailable distinct; null is not zero. Accepted/no-retry requests or
low cost prove no competence; sparse scores cannot justify confidence.

Use branches.suggestion and returned rules/counts. Its full retained active-revision evidence and
post-announcement upward cohort cannot be replaced by windowed rows/latest samples. Announcements
prove no executed version. Step up lowers the threshold/more Regular; step down raises it/more
conservative. plan-trust replay estimates historical allocation without
model calls, not competence for newly Regular work. Do not recreate backend suggestion policy.

Check pending/per-Plan unavailable; failure is not healthy emptiness. Use bounded read handling below.

### Ongoing follow-up

Offer follow-up. When requested, reuse an Agent/host scheduler and matching job (Hermes cronjob is one example).
Record verified job ID, cadence/timezone and authorized destination. Prefer daily unless requested
otherwise, not minute-level full scans. Fresh self-contained runs load this Skill, use absolute CLI
path/target/context/Plan IDs and read fresh reports. Keep minimal comparison notes outside this Skill;
compare like revision/model/reasoning/cohort scopes and disclose changes. Honor regular-report requests;
otherwise notify changed advice, meaningful competence/protection changes or new read gaps, staying
quiet on unchanged healthy/insufficient evidence. Unverified scheduling
remains unconfigured, with manual read commands; monitoring grants no routing-edit authority.

## Analyze history and savings

Same-OS-user content/search needs no GUI ticket. Global --agent selects sealed collaboration, not
management. Supplied grants are checked; model keys grant no management access. Start one scope:

```sh
hiroute value show --view summary --period 7d
hiroute value show --group-by session --order-by cost-desc --period 7d
hiroute sessions list --view switches --period 7d
hiroute sessions list --view conversations --period 7d --query TEXT
```

Use sessions/value help. Disclose requested/effective bounds, timezone and clipping; civil days are
service-local. Keep analysis_scope for related/narrower reads without new time selectors. Search
starts with --query TEXT; changing keywords needs a new scope. Page unchanged view/filters/limit to
END; member previews/page lengths are not totals:

```sh
hiroute sessions show SESSION_ID --view routing --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --analysis-scope TOKEN
```

For request cost use value show --group-by request/--session; add --request to conversation reads for
exact request content, retaining --analysis-scope. Keep scope across requests/compaction/follow-ups.
Traffic/settlement/prices cannot change pinned results. Never silently refresh or compare cross-scope
totals. Expiry, withdrawn proof, deletion/authority changes need a fresh initial query; give its command.

Explain recorded decisions, served model/reasoning/publication. Stages are not switches: between_turn
needs adjacent completed user turns; continuation_boundary can be mid-turn compression;
turn_context_reset/pending/unknown proves no adjacency. Missing lineage separates members; matching
text/time proves no link. Key relay/fallback is not a turn switch. Count requests/attempts once.
For reference_only/partial use content_id/request_id, sessions.show schema and V2 catalog/content pages;
retain cursor/scope and minimal excerpts, excluding private provider state. sessions receipt is immutable
evidence; plan-quality samples explain stages. Legacy list --include-unlinked --query and show
--content messages-and-tools remain supported. Use public reads, not manual database joins.

For pending/index_unavailable show progress/coverage/stall. For timeout/reader_busy follow
error.observation_read/retry_after_ms with bounded retries, then disclose failure; narrow request/page
where supported. Missing/deleted/expired capture alone proves no routing error. Compare task/answer/tools with classifier/group/probability/threshold/competence/reasoning before proposing changes.

Ranking needs no reference. Choose one currency/recorded valuation kind or --basis api-equivalent for
current prices. Summary defaults to that estimate and an unambiguous primary model; resolve returned
choices with real binding IDs. Keep currencies, valuation kinds and historical/current prices separate.
Unknown is not free/zero. Disclose highest-known-only rankings, missing coverage and signed paired
savings; zero denominator has no percentage. Decision overhead is unmeasured. Coding Plan estimates
are API-equivalent, not cash bills or quality proof.

Missing prices: prices effective schema needs Binding/currency/valuation_kind=api_equivalent. Use
edit_context in prices override preview and exact Apply/Operation, with supported per-million-token
rates, not invented prices/FX/usage. Equal cache-creation/input rates remove estimate sensitivity,
not measurement gaps. Keep complete before.setting and filled restore Preview/Apply (follow_catalog removes override).
Re-estimation changes no entitlement/history; free/subscription/paid/unknown differ.

## Exact submission and scope

--apply-file creates private exact request/key files without secrets/capabilities. Review effects;
submit submission.arguments; retain file/operation_lookup. Use the OS socket, not minted capabilities.
Uncertain delivery: operations find/get or replay the **identical file/key**. Confirmed unaccepted stale
input needs fresh Preview. Reuse completed IDs. Keep authorization/notes outside this Skill; honor
scope/cancellation/quotas. Quickstart/status never call models.

## Worker

Use worker help: worker dependencies discover --harness codex_cli, then worker dependencies select
--request-stdin. Select a complete tuple using selection_revisions[].revision as
expected_selection_revision, not a CLI/package version. Selection installs nothing. Read executors/plans.
worker exec needs a published delegation-enabled Plan, absolute cwd, text source and stable key.
Retain submission/run ID for status/wait/result/list; wait timeout is pending. Replay the identical request/key after uncertainty; cancel only on request. Management grants no recursive or coordination/
status/result-summary delegation. Readiness proves no success.

## Listener and service changes

Read gateway show before set; remote/wildcard needs explicit risk acknowledgement. Distinguish applied/
ready from firewall/TLS. gateway recover restores the safe listener. Verify Local Control readiness
after start/restart; start is not autostart.
