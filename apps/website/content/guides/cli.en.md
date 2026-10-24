# HiRoute CLI

HiRoute CLI is the official terminal entry to the HiRoute local service. It can connect to Desktop or serve as the complete management interface for Linux headless. Model sources, smart routes, Agent connections, session observation, and Worker tasks reuse the same production paths as Desktop; there is no second headless control plane.

## Install and inspect the CLI

- macOS Desktop: open Settings → CLI → Terminal entry and select Install.
- Linux headless: follow [Run HiRoute headless on Linux](/en/docs/install-linux/) to use the website's one-line installer, then start the service explicitly.

If the entry directory is not on PATH, add this line to your shell configuration and open a new terminal:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Inspect the entry and released commands:

```sh
hiroute --help
hiroute schema list --output json
hiroute schema show --command-id worker.exec --output json
```

The CLI entry and local-service availability are separate states. If a command reports that the service is unavailable, Desktop users should open the app. Standalone users should run `hiroute service status`, then `hiroute service start` when needed. The CLI never starts the service or replays a failed request automatically.

## CLI Quickstart: connect, inspect, and adjust

The same path works from a terminal or any Agent with the HiRoute management Skill. The caller can manage a different Agent; its own model need not use HiRoute. Model routing, management and task delegation remain independent.

```sh
hiroute quickstart
hiroute quickstart --context CONTEXT_ID --plan PLAN_ID
```

Quickstart is read-only: it shows the current service, exact targets, reusable sources/Plans, missing choices and next commands. It does not start a service, import credentials, save changes, restart an Agent or call a model. For a specific Hermes configuration, add `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`. Preserve a missing selected root; do not silently substitute another profile.

1. Reuse a saved source from `compute list`, or scan a connection point and explicitly prepare its credentials with `compute connection prepare`. Use `compute scan --request-stdin` with `{"schema":"hiroute.compute-scan-request/v2"}`; add the selected typed `native_target` for Hermes. The returned candidate contains its exact reference/revision and selectable models.
2. Preview the selected models, then save the generated request:

   ```sh
   hiroute compute connection preview --candidate CANDIDATE_REF --candidate-revision REV --all-selectable --apply-file source-apply.json
   hiroute compute connection apply --request-file source-apply.json
   ```

   Use repeated `--model-ref REF` for a subset. `--all-selectable` freezes concrete current refs; zero selectable models blocks saving. Preparation, save, provider checks and real native requests are different effects.
3. Select returned binding IDs with `routing options --request-stdin` and `{}`. Create a fixed route with `--mode fixed_model --model BINDING_ID`, or go directly to difficulty routing:

   ```sh
   hiroute routing preview --name 'Economy work' --purpose 'Routine and complex development' --mode smart_saving --economy ECONOMY_BINDING --primary PRIMARY_BINDING --decision local_rules --apply-file route-apply.json
   hiroute routing apply --request-file route-apply.json
   ```

   Replace all example IDs with current returned IDs. Repeat group flags for explicit order. Choose `--decision local_rules` or `--decision-service ID@REVISION` explicitly; an ordinary Chat model is not a decision-service connection. Shared backend defaults fill the editor. To change one choice, use `routing preview --plan PLAN_ID` with only that change and a new Apply file; other saved settings are preserved.
4. Run `agents connect preview --context CONTEXT_ID --plan PLAN_ID` with the native choices from `--help`, and add `--apply-file connect-apply.json`. Hermes requires an explicit protocol; Codex requires native-model/default choices; Claude requires selected presets; Qoder/Pi/DSH add entries without claiming a global default. `--management enable` and `--collaboration explicit|delegate_by_default` configure independent Skills; neither needs caller model routing. Review the result, then run `agents connect apply --request-file connect-apply.json`.
5. Inspect the Operation and `agents connect status --effective --context CONTEXT_ID --plan PLAN_ID`. Follow the returned native activation/selection guidance. Saved files do not prove loading, and a provider check does not prove the Agent used the route. Without consent for a model call, verification remains pending. A matching request identifies its route, actual model, outcome and reported usage only for that request/session. Unknown price is not zero cost.

Apply files are new private files, never overwritten, and contain exact requests and stable keys. Keep `submission.operation_lookup` for `operations find`, or replay the identical Apply file after uncertain delivery. Do not reconstruct a new request/key to retry accepted work. Rerun Quickstart to reuse completed steps. To restore only one capability, use `agents restore preview --context CONTEXT_ID --facet management_skill --apply-file restore-apply.json`, then `agents restore apply --request-file restore-apply.json`; `model` and `collaboration` select the other facets. Restore preserves unrelated data and may leave native loading/selection to the user.

Use family/leaf help for branches and `schema show --command-id routing.preview --include-schema` for complete installed definitions. The unflagged schema command remains a descriptor. JSON input and convenience flags are alternatives; `--apply-file` may accompany either supported Preview form. Desktop resident-service prerequisites must be completed through Desktop, not a fabricated CLI declaration.

Business output is one complete JSON document on stdout. Capture stderr separately and save the real exit code; an accepted Operation is not completed work. In a terminal tool that merges streams:

```sh
(
  hiroute quickstart > quickstart.stdout.json 2> quickstart.stderr.txt
  hiroute_exit=$?
  printf '%s\n' "$hiroute_exit" > quickstart.exit.txt
  exit "$hiroute_exit"
)
```

Read the whole JSON file, diagnostics and saved code separately. Never parse only the first line or use a later shell command's status. Exit 2 is invalid input; `--error-details` adds bounded field diagnostics. Keep personal notes outside the managed Skill file.

### Connect a decision service from the CLI

Run `hiroute decision services options` before creating a connection. Reuse an explicitly selected
saved `ID@REVISION`; a saved record is not a connectivity test. Installed presets include Bailian
Token Plan/workspace, OpenRouter and TypeSafe, plus System One compatible and custom extensions.
They describe integration defaults, not current provider availability, account entitlement or price.
Workspace/compatible entries require the endpoint/model shown in their options. Ordinary Chat
execution models are not decision services. Local rules remain an explicit alternative.

A credential is a human step outside chat. In your own terminal on the HiRoute host (for example
an SSH session), run the following with a fresh non-secret candidate reference:

```sh
hiroute protected-input register --candidate CANDIDATE_REF --prompt --format bearer
```

Paste only the API key into the hidden prompt. HiRoute adds `Bearer` and returns registration
status. Never give a key to an Agent through chat or a terminal tool, arguments, environment or
ordinary JSON/files. Use `--format raw` for a custom full header value. Cancellation or no controlling
terminal saves no service. Tell the managing Agent only whether registration succeeded, then
continue with the same reference. Desktop's existing password field is also supported.

```sh
hiroute decision services apply --id daily-decision --name 'Daily decisions' --provider typesafe --candidate CANDIDATE_REF --apply-file decision-apply.json
hiroute decision services apply --request-file decision-apply.json
hiroute decision services test --service daily-decision@SAVED_REVISION
hiroute routing preview --plan PLAN_ID --decision-service daily-decision@SAVED_REVISION --apply-file decision-route-apply.json
hiroute routing apply --request-file decision-route-apply.json
```

Choose the actual preset; provide `--endpoint` and `--model` where required. A custom connection
uses `--provider custom --endpoint URL` with a protected candidate and optional `--header NAME`,
or explicit `--no-auth`. New connections require an exact ID and name; edits use `--id` and preserve
omitted fields. Endpoint changes require new credential input or explicit custom no-auth. Review
each Preview and inspect its Operation; use the returned saved revision, not the placeholder.
The exact private Apply file and lookup identity support identical retry. Keep completed source/Plan
steps and release unused candidate references.

The explicit synthetic test may incur provider charges. Inspect `data.outcome` and `failure_code`:
a successful CLI query can report a failed connection test. Passing proves that probe, not semantic
quality, actual execution-model access or native loading. After publishing, verify the actual decision
source, execution group, model and outcome in matching requests. Manual model substitution does not
prove a complex decision. Empty custom keywords do not disable built-in local rules; report unmatched
natural tasks honestly. Include cancelled and transport-failed requests in results.

Before restoring or reconnecting the managing caller's own model route, establish independent native
access or an external operator handoff. A successful restore can revoke the grant still held by a
running process. Restored files do not prove reload; arrange continuation before that effect.

## Two command contracts

Discover Host management commands through root and family help:

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
```

Discover Application / Local Control commands through the Released schema and complete leaf help:

```sh
hiroute schema list --output json
hiroute schema show --command-id routing.apply --include-schema --output json
hiroute routing apply --help
```

Do not expect `schema list` to contain Host commands, and do not infer fields for the installed version from a website snippet.

## Manage models, routes, and Agents

The Released CLI now supports the complete headless loop:

- Discover and inspect model sources with `compute scan/list/show`; check and save connections with `compute connection options/test/preview/apply/authorize`.
- Inspect candidate model capabilities with `models show`.
- Manage saved decision model or custom extension revisions with `decision services list/apply/test`.
- Create, update, and publish smart routes with `routing options/list/show/preview/apply`.
- Discover local Agents with `agents scan/list/check`; connect with `agents connect preview/apply/status` and recover with `agents restore preview/apply`.
- If a write response is lost or uncertain, query the operation in its original idempotency domain with `operations find/get`.

Configuration writes preview first, then apply with the complete request, preview digest, exact revisions, and idempotency key. Models, routes, and Agents use their own `preview/apply` commands. Decision connections use `decision services apply` for both preview and save. Pass passwords and API keys through `protected-input`, never through ordinary JSON, arguments, or logs. Obtain exact fields from the matching `schema show`, `options`, and leaf `--help`.

## Manage decision connections

The CLI keeps the `decision services` command name; Desktop places these connections under Models → Decision models. Built-in decision models and custom extensions are independent of general models that execute tasks:

```sh
hiroute decision services list --output json
hiroute schema show --command-id decision.services.apply --output json
hiroute decision services apply --help
hiroute decision services apply --request-stdin --output json < decision-preview-request.json
hiroute decision services apply --request-stdin --output json < decision-apply-request.json
hiroute decision services test --request-stdin --output json < decision-test-request.json
```

There is no separate `decision services preview` command. The first `apply` accepts `{schema_version, spec}` and only previews. To save, send the same `schema_version`, returned `data.normalized_spec`, `accept_digest` from `data.change_digest`, exact `data.expected_revisions`, and a stable idempotency key. `spec.desired_state` contains the connection ID, `expected_revision`, complete `service`, and a protected `input_slot` when supplying a new credential. `service: null` deletes an unreferenced connection.

The list returns each connection's latest revision. A route selects a complete saved revision, checked by ID, revision, and content; historical revisions remain publishable. Difficulty routing uses editor `smart.classifier` and `smart.judgment`. Custom branches uses `branch_routing.classifier`, plan `branch_routing.judgment`, and each branch's regular `candidates`, `primary_candidates` (an empty array when no primary group is configured), and optional complete `judgment` override. An omitted override follows the plan; copying the whole set makes it independent, and clearing it restores defaults.

Saving r2 does not change a route pinned to r1. Select the new revision and publish through `routing preview/apply`. A test request uses `hiroute.classifier-decision-test/v1` with `classifier: {kind: "decision_service", service: <complete saved revision>}`. It sends fixed synthetic input to that exact connection revision, reads no real conversation, and produces no quality sample. Check `data.outcome` and `data.failure_code`; command success alone does not mean the test passed.

Custom extensions implement the [Custom extension API](/en/docs/decision-api/). See [Connect decision models](/en/docs/decision-extensions/) for built-in provider configuration and interface mapping. The optional [self-hosted Jev reference extension](/en/docs/jev-decider/) provides deployment examples. See [Use smart model routing](/en/docs/model-routing/) for the Desktop configuration steps.

## Inspect sessions and runtime performance

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show <SESSION_ID> --output json
hiroute sessions receipt <RECEIPT_ID> --output json
hiroute sessions status --output json
hiroute value show --routing <PLAN_ID> --session <SESSION_ID> --output json
hiroute observation plan-quality samples --plan-id <PLAN_ID> --output json
hiroute observation plan-quality samples --session-id <SESSION_ID> --limit 50 --output json
```

Session queries return facts and a timeline by default, not conversation bodies. A receipt reports the actual route, model, and upstream-reported tokens. When no trustworthy price evidence exists, monetary value remains unknown instead of being fabricated as zero.

`observation plan-quality samples` requires at least a plan or session scope and returns the stage facts used by runtime performance. `branch_execution` records the actual task branch, model group, candidate position, and judgment policy at execution. The turn's selection reason and later stage assessment are stored separately. Use `--competence below-floor|meets-floor` to compare with the saved competence floor, or `--unrated` for missing or partial scores; unrated is not zero. Use the returned cursor for more pages. This query calls no model and returns no protected conversation bodies.

## Discover executors and plans

```sh
hiroute worker executors
hiroute worker plans
```

`worker executors` lists execution agents and their availability on this device. `worker plans` lists published plans allowed for the current agent. Choose an agent that passes dependency checks and use the returned plan ID, not its display name.

## Start a task

```sh
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --title "Investigate failing tests" \
  --submission-key my-check-001 \
  -- "Find the cause, propose the smallest fix, and run the relevant checks"
```

Supply exactly one input source: text after `--`, a `--file`, or standard input. `--cwd` is converted to an absolute path, but it is not directory authorization or a concurrency lock.

`--submission-key` is a caller-selected idempotency key. If a connection or process interruption leaves acceptance uncertain, keep the same key and query it. Do not generate a new key and retry blindly:

```sh
hiroute worker status --submission my-check-001 --operation start
```

## Read progress and results

Use the run ID returned at admission:

```sh
hiroute worker status --run <RUN_ID>
hiroute worker wait --run <RUN_ID>
hiroute worker result --run <RUN_ID>
```

`wait` is bounded and never cancels a task that is still running. `result` supports offset and maximum-byte options for paging through a large result.

## Continue or cancel

Continuation requires both the task ID and its exact latest run ID:

```sh
hiroute worker continue \
  --task <TASK_ID> \
  --expected-latest-run <RUN_ID> \
  --submission-key my-check-002 \
  -- "Finish the fix based on the test result"
```

Cancel one exact run:

```sh
hiroute worker cancel --run <RUN_ID> --reason user-requested
```

Cancellation does not undo files or external effects already produced.

## Machine-readable output

Management business commands default to JSON and accept JSON output. Worker commands support their documented `text|json|quiet` modes and normally default to text; use `--output json` for automation. Check each installed leaf help for its actual modes. `schema show --include-schema` adds full definitions for supported management commands; the unflagged form is a descriptor.

CLI and daemon accept only the currently Released business commands and continue to reject Planned commands. Automation should discover the runtime schema instead of hard-coding the command count or unreleased capabilities.

## Recover the complete Plan configuration

Before modifying an existing Plan, retain its complete successful read in a fresh private file:

```sh
(umask 077; set -C; hiroute routing show PLAN_ID --output json > /ABS/plan-before.json)
```

Check the actual exit code and whole response. Before applying a change, provide the user with
recovery commands populated with the installed CLI's absolute path, real Plan ID and unique absolute
files. The commands remain usable from an independent terminal if the managing Agent cannot continue:

```sh
hiroute routing preview --plan PLAN_ID --restore-from /ABS/plan-before.json --apply-file /ABS/restore-apply.json
hiroute routing apply --request-file /ABS/restore-apply.json
hiroute routing show PLAN_ID --output json
```

The first command only previews: it reads the saved public response, uses the current head revision
and existing complete editor projection, and prepares the normal exact Apply file. Do not combine
`--restore-from` with individual edit flags or typed request input. Invalid, unsuccessful or
different-target responses are rejected. Missing dependencies and revision conflicts stay explicit.
After uncertain delivery, inspect/replay the same Apply; a certainly stale request needs a fresh
Preview and new file. Never bypass revision checks.

Compare every restored `data.desired` field, including candidate order/reasoning, decision service and
judgment, context/limits, requirements, mode/delegation/work, plus the alias. Revision/publication
numbers advance. Replacing a candidate can select role defaults; choosing its old model again does
not restore custom reasoning. Sources, credentials and native Agent facets retain their own recovery
owners: a saved Plan response is not a whole-installation backup.

For native settings provide the matching concrete `agents restore preview --context ... --facet ...
--apply-file ...` and `agents restore apply --request-file ...` commands, with actual native-root or
selection flags and the owned restore reference. Arrange independent access before a restore/reconnect
can revoke the caller's grant. Native loading/selection remains a separate reported action.


## Analyze historical conversations and estimated API savings

The installed CLI uses the authenticated same-OS-user Local Control socket. Explicit content and search reads do not need a separate Desktop-created read grant. A supplied grant is still validated; a model Gateway key grants no management access. Start with:

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show SESSION_ID --content messages-and-tools --output json
hiroute sessions receipt RECEIPT_ID --output json
hiroute observation plan-quality samples --plan-id PLAN_ID --output json
```

Use returned IDs. For bounded request details, `schema show --command-id sessions.show --include-schema` describes the existing V2 timeline, catalog, ancestry and content queries. `sessions list --request-file QUERY_JSON` supports V2 search/session_search; `sessions receipt --request-file QUERY_JSON` supports request facts. Follow `next_cursor` with the same time/filter scope. Correlate task text and tool/answer history with decision source, simple probability/threshold, selected group, competence assessment, actual model/reasoning and publication. Missing capture, expired data and pending indexes are coverage gaps. Review proposed policy changes and retain the complete recovery commands above before applying them.

Coding Plan/subscription payments are separate from API-equivalent value. Choose an actual reference Binding, normally the Plan primary model, and a currency:

```sh
hiroute value show --routing PLAN_ID --period 7d --basis api-equivalent --reference-binding PRIMARY_BINDING --currency CNY --output json
```

This opt-in view values recorded usage with **current source-specific API-equivalent prices**, including observed retry attempts. It reports the reference and price provenance, evaluation time, routed/reference estimates and signed estimated savings. It does not rewrite frozen historical receipts or establish equal output/quality, a subscription bill or cash savings. Partial/unknown totals stay null; paired subtotals explicitly count their complete comparable requests. Negative savings remain negative. Retention and unpriced/missing usage reduce coverage; narrow a large interval if the bounded query is unavailable. Day grouping is unavailable in this view. Ordinary `value show` retains its historical meaning.

For missing rates, read the installed `prices.effective` and `prices.override.preview` schemas. Query the exact Binding locator with currency and `valuation_kind: "api_equivalent"`; copy the returned `edit_context` revisions into a price Preview. Manual `rates` use integer currency micros per million tokens, with separate input/output/cache rates. Use verified prices or ask for these non-secret values; do not invent them. Apply the exact returned spec/digest/revisions with one retained key. Before changing a price, retain its prior setting and give fully populated price-restore Preview/Apply commands; `follow_catalog` restores catalog lookup when no manual override existed. API-equivalent prices can be set for subscriptions without changing their entitlement or ordinary billing class. Free, subscription, paid and unknown remain distinct.

When cache creation is charged at the ordinary input rate, set its reference rate equal to input. The amount then does not depend on an unreported input/creation split; HiRoute can combine equal-rate calculation buckets without filling or rewriting recorded usage. Different rates still require the missing count.

Custom extensions return category and group directly. Their route editor hides HiRoute thresholds and rubrics while retaining parked edits; optional scores are observation-only.
