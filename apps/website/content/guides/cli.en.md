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
hiroute schema show --command-id routing.apply --output json
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

The list returns each connection's latest revision. A route selects a complete saved revision, checked by ID, revision, and content; historical revisions remain publishable. Smart saving uses editor `smart.classifier` and `smart.judgment`. Custom branches uses `branch_routing.classifier`, plan `branch_routing.judgment`, and each branch's regular `candidates`, `primary_candidates` (an empty array when no primary group is configured), and optional complete `judgment` override. An omitted override follows the plan; copying the whole set makes it independent, and clearing it restores defaults.

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

The results contain the execution agents available on this device and the published plans allowed for the current agent. Do not infer a plan ID from its display name; use the ID returned by the command.

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

Public commands support `--output text|json|quiet`. Use the default `text` interactively, `json` for scripts and main agents that consume the schema, and `quiet` when only the exit result matters. Use `hiroute schema list` and `hiroute schema show` to discover the current machine contract at runtime.

CLI and daemon accept only the currently Released business commands and continue to reject Planned commands. Automation should discover the runtime schema instead of hard-coding the command count or unreleased capabilities.
