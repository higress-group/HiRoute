---
name: hiroute-management
description: Manage HiRoute through the installed CLI.
metadata:
  schema: hiroute.management-skill/v1
  access: local-user
---

<!-- Managed by HiRoute standalone installer -->

# HiRoute management

Use installed commands; distinguish caller, target and independent model/management/collaboration facets.

## Target and contracts

Use quickstart/help and current context/Plan IDs; resolve ambiguity. Hermes needs --hermes-root
/ABS/ROOT. Saved/loaded/selected/served differ. Install CLI via Desktop Settings → CLI or standalone;
Agent → HiRoute management enables this Skill independently of model/delegation.
Keep one daemon; use status/doctor/bounded logs. version --output json and hirouted --version identify
their own build; missing compiled SHA is unknown.

Use family/leaf --help and schema show --command-id COMMAND --include-schema; fall back to descriptor/
leaf help. Planned handlers are unavailable. Resolve IDs/revisions/choices from current results. Use --output json for business reads/writes. Capture complete stdout JSON, separate stderr and real exit; never merge streams or parse just the first line. Exit 2 is input error; --error-details gives hints. Accepted is
pending; query success may contain a failed Operation.

## Model access

Reuse compute list sources; otherwise compute scan --request-stdin with
`{"schema":"hiroute.compute-scan-request/v2"}` and selected Hermes native_target discovers endpoints.
Use refs/revisions, stable prepare_id and installed prepare schema; preparation rereads credentials privately. Refs/catalogs prove no save/entitlement/inference; never patch native config.

Select all selectable models unless the user requests a subset:

```sh
hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source.json
hiroute compute connection apply --request-file source.json
```

--model-ref selects a nonempty subset, never a future wildcard. Drift needs fresh preparation. Secrets
use protected FD/candidates (FD >= 3) or the human terminal, never argv/env/ordinary JSON/files/chat.
Release unused refs; never replay uncertain one-use subscription callbacks.

Networking defaults to Direct. compute network show/resolve is non-probing; preview/apply needs current
revisions/exact retry key. Changes affect new requests; Inherit uses startup env. Subscription settings differ.
Read connection test-network/decision network test help for identity. Tests may bill; inspect
check/network/failure. Route changes stale success; proxy auth does not justify Key replacement/disablement.

## Create or edit routing

routing options --request-stdin with `{}` gives IDs/defaults. Fixed uses --mode fixed_model
--model BINDING_ID. Difficulty routing keeps the smart_saving CLI identifier:

```sh
hiroute routing preview --name 'Routing' --purpose 'Route tasks' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route.json
hiroute routing apply --request-file route.json
```

Repeat group flags for order. Choose local_rules or saved --decision-service ID@REVISION explicitly;
Chat is not the decision protocol. Edit with --plan PLAN_ID and requested fields/new file; omissions
remain. Judgment threshold/floor: integer thousandths, 0..1000.

### Complete Plan recovery

Before edits, save a private complete baseline; check exit/response:

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/before.json)
```

Before Apply fill **actual CLI absolute path, Plan ID and unique absolute files**; model names alone cannot restore defaults:

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/before.json --apply-file /ABS/restore.json
hiroute routing apply --request-file /ABS/restore.json
hiroute routing show PLAN_ID --output json
```

Compare order/reasoning, classifier/judgment, context/limits, requirements, mode, delegation/work
and alias; inspect Preview/Operation. Restore advances current head. Never mix
restore/edits, copy revisions or substitute refs. Later recovery needs fresh Preview/Apply identity;
sources/credentials/Agent facets recover separately.

### Decision-service onboarding

decision services options lists saved ID@REVISION and provider/custom choices; presets prove no availability/test. Credentials need a human handoff **outside chat**: give a fresh non-secret
candidate ref and this command for the user's terminal:

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

Never run the prompt in Agent tools. It reads a hidden bare key and adds Bearer; receive only status/ref.
Full headers use --format raw; no auth needs --no-auth. Cancel/missing terminal saves nothing;
restart needs fresh input/ref. Desktop/protected handles replace the terminal.

Read decision services apply help for provider/endpoint/model/candidate. Prepare --apply-file, review,
apply --request-file, inspect Operation, then decision services test --service ID@SAVED_REVISION. Bind
that revision with routing preview --plan PLAN_ID --decision-service ID@REVISION and Apply. Edits use
--id/requested fields; omissions remain. Endpoint changes need explicit credentials/no-auth. Keep retry files; release refs. Inspect billed data.outcome/failure_code; probes prove no quality/native loading.

Verify classifier/group/model/publication/reasoning and request outcomes. Swaps prove no complex-branch
coverage; empty keywords keep built-in rules. Report misses/cancellation/timeouts; transport failure is not delivery.

## Connect or restore Agent facets

Before restore/reconnect/grant edits, ensure independent caller continuity; processes may retain old grants.
Unknown dependency stays pending for an operator.

Unknown Codex/Claude Skill loading needs agents check AGENT_ID --scope collaboration, then fresh
Preview. This isolated native/local challenge makes no upstream call; failed prerequisites remain.

agents connect preview --context CONTEXT_ID --apply-file PATH takes requested facets: --plan PLAN_ID,
--management enable, --collaboration explicit|delegate_by_default. Omissions remain; management-only
needs no Plan/plugin. Delegation requires user intent/collaboration policy.

Keep model plugins on binary/Skill upgrades. Refresh the saved model facet with access_token=keep;
reload/start native as needed, preserving other facets. Hermes needs exact root/protocol and Responses
reasoning; >=0.21.7 is recommended for replay, but compatible older plugin/SDKs remain allowed. No
unsolicited upgrade/patch/thinking disable. Codex needs exact root/profile/native model; preserve other
Claude presets. Qoder/Pi/DSH entries claim no global default.

Honor blockers/native prerequisites; inspect Operation and:

```sh
hiroute agents connect status --effective --context CONTEXT_ID --plan PLAN_ID
```

Session selection may override config; no global IM switch is proved. Match target/request/
publication/model/outcome; absent loading/model-call consent leaves verification pending.

Restore owned facets using retained refs; preserve foreign edits and caller continuity:

```sh
hiroute agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore.json
hiroute agents restore apply --request-file restore.json
```

Repeat --facet for model/collaboration or a set. Fill root/selection flags; give native follow-up and
inspect conflicts. Backups prove neither restoration nor reload.

## Review competence and routing suggestions

Resolve authorized Plan/context IDs:

```sh
hiroute observation plan-trust suggestions --output json
hiroute observation plan-trust --plan-id PLAN_ID --revision active --group-by model --output json
```

Filter to selected Plans. Show revision, branch/model/reasoning, interval/cohort, scored/executed/session
counts and competence/coverage/protections. Keep partial/unscored/awaiting/unavailable distinct; null is
not zero. Acceptance/no retry/low cost proves no competence; sparse scores justify no confidence.

Use branches.suggestion rules/counts, full active-revision evidence and post-announcement upward cohort,
not windowed rows/latest samples. Announcements prove no execution.
Step up lowers threshold/more Regular; step down raises it/more conservative. plan-trust replay
estimates allocation without model calls or new Regular competence; keep backend policy.

Check pending/per-Plan failures; apply bounded read handling below, never report healthy emptiness.

### Ongoing follow-up

Offer follow-up; on request reuse Agent/host scheduler and matching job. Verify job ID, cadence/timezone
and authorized destination; default daily, not minute-level full scans. Runs load this Skill,
absolute CLI/target/context/Plan IDs and fresh reports. Keep minimal notes outside this Skill; compare
matching revision/model/reasoning/cohort and disclose changes. Honor regular reports; otherwise notify
changed advice, meaningful competence/protection changes or read gaps. Stay quiet on unchanged healthy/
insufficient evidence. Unverified schedules stay unconfigured; give manual reads. Monitoring grants no edit authority.

## Analyze history and savings

Same-OS-user content/search needs no GUI ticket. --agent selects sealed collaboration; model keys grant
no management access. Check supplied grants. Start a scope:

```sh
hiroute value show --view summary --period 7d
hiroute value show --group-by session --order-by cost-desc --period 7d
hiroute sessions list --view switches --period 7d
hiroute sessions list --view conversations --period 7d --query TEXT
```

Disclose requested/effective bounds, timezone and clipping; civil days are service-local. Keep
analysis_scope for related/narrower reads without new time selectors. Search starts with --query TEXT;
changing keywords needs a new scope. Page unchanged view/filters/limit to END, not a presumed total:

```sh
hiroute sessions show SESSION_ID --view routing --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --analysis-scope TOKEN
```

Request cost: value show --group-by request/--session; conversation --request reads one request.
Keep --analysis-scope across requests/compaction/follow-ups. Traffic/settlement/prices cannot change
pinned results; never silently refresh or compare cross-scope totals. Expiry/withdrawal/deletion or
authority changes need a fresh initial query; give its command.

Explain served model/reasoning/publication and recorded decisions. Stages/key relay/fallback are not
turn switches. between_turn needs adjacent completed user turns; continuation_boundary may be mid-turn
compression; turn_context_reset/pending/unknown proves no adjacency. Missing lineage separates
members; matching text proves no link. Count requests/attempts once. For reference_only/partial use
content_id/request_id, sessions.show schema, V2 catalog/content pages and minimal excerpts; exclude
provider private state. sessions receipt is immutable; plan-quality samples explain stages. Legacy
list --include-unlinked --query and show --content messages-and-tools remain. No database joins.

Show progress/coverage/stall for pending/index_unavailable. For timeout/reader_busy use bounded
error.observation_read/retry_after_ms retries, then disclose failure; narrow where allowed.
Missing/deleted/expired capture proves no routing error. Compare task/answer/tools with
classifier/group/probability/threshold/competence/reasoning before changes.

Rankings need no reference. Choose one currency/recorded valuation kind or --basis api-equivalent for
current prices. Summary defaults to that estimate and an unambiguous primary model; resolve returned
choices with real binding IDs. Separate currencies, valuation kinds and historical/current prices.
Unknown is not free/zero. Disclose highest-known-only rankings, missing coverage, signed paired savings,
no percentage for zero denominator and unmeasured decision overhead. Coding Plan API-equivalent
estimates prove neither cash savings nor quality.

Missing prices: prices effective needs Binding/currency/valuation_kind=api_equivalent. Use edit_context
with prices override preview and exact Apply/Operation, supported per-million-token rates and no
invented prices/FX/usage. Keep complete before.setting and filled restore Preview/Apply (follow_catalog removes
override). Equal cache-creation/input rates remove sensitivity, not measurement gaps. Re-estimation
changes no entitlement/history; free/subscription/paid/unknown differ.

## Exact submission and scope

--apply-file creates private exact requests/keys without secrets/capabilities. Review effects; submit
submission.arguments; keep file/operation_lookup. Use the OS socket; never mint grants. After uncertainty use operations
find/get or the **identical file/key**; confirmed unaccepted stale input needs fresh Preview. Reuse
completed IDs. Keep authorization/notes outside this Skill and honor scope/cancellation/quotas.
Quickstart/status never call models.

## Worker and collaboration

worker exec needs no collaboration Skill; standing policy requires user intent.
Enable a Plan; new Plans also need routing choices:

```sh
hiroute routing preview --plan PLAN_ID --delegation enable --worker-harness codex_cli --worker-protocol responses --apply-file work.json
hiroute routing apply --request-file work.json
```

Choose Harness/protocol; missing work returns choices. Omit both Worker flags to keep work;
--delegation disable keeps work but stops new delegation. Preview never publishes.

Run worker dependencies discover --harness codex_cli then worker dependencies select --request-stdin.
Use found tuples: expected_selection_revision = selection_revisions[].revision, not a CLI/package version.
Discovery installs nothing; read executors/plans. worker exec needs absolute cwd, text and
stable key; exec/continue use approve-all, omit permission options. Keep submission/run IDs for
status/wait/result/list. worker wait --run ID blocks until the run ends and includes an available
successful result; run it in a notifying background terminal when the main Agent must stay responsive.
--wait-timeout 1..30 bounds total waiting; --after-revision N watches one revision with a 30-second
default. Use status for an immediate query. Wait timeout/interruption never cancels work.
Replay identical request/key after uncertainty;
cancel only on request. Readiness proves no success.

Standing policy: preview agents collaboration configure AGENT_ID --trigger explicit --preview; to save,
omit --preview; set --idempotency-key KEY. explicit delegates only when asked; delegate-by-default
permits suitable delegation within the user's task. This installs hiroute-collaboration independently
of models/management. Read agents collaboration status AGENT_ID; remove via owned facet Restore. Reload Skills in a fresh session if needed. No recursive or coordination/status/result-summary delegation.

## Listener and service changes

Publish Plans/runtime settings via CLI/Operation; check completion/effective readback. They apply live;
follow native Agent selection/reload separately.

Read gateway show before set; acknowledge remote/wildcard risk. gateway set/recover owns listener restart;
recover restores the safe listener. Applied/ready proves no firewall/TLS.

Plan/settings/listener changes need no manual restart. Before necessary binary/environment restarts,
ensure independent operator continuation if caller model access uses HiRoute. Before model verification
require service status local_control_ready=true, system status daemon=role_all/gateway=ready and
gateway show ready=true. Manager active/exit zero or sleep is insufficient; start is not autostart.
