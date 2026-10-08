# HiRoute Linux headless CLI

[简体中文](standalone-cli.zh-CN.md)

Standalone is the single-user mode that does not install Desktop. The production `hiroute`
CLI, role-all `hirouted` daemon, Local Control, Gateway, business storage, and observation
form one control and execution loop. It uses the same-UID local trust boundary, needs no
second “CLI management token,” and exposes no second Agent-management API.

Standalone and HiRoute Desktop must not manage the same user installation at the same time. It does not
provide a system-wide multi-user install, a Windows installer, runtime downloads, or automatic
updates. macOS can use the same candidate-package mechanism, but this page and the Quickstart
are validated for Linux.

## Installation

Install the latest stable Linux package for `x86_64` or ARM64 (`aarch64`) from
[hiroute.ai](https://hiroute.ai/en/download/):

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
hiroute service start --output json
hiroute system status --output json
```

The installer verifies the package and installs for the current user. It does not start
the service automatically. See the [Linux installation guide](https://hiroute.ai/en/docs/install-linux/)
for host prerequisites and service management.

## Build a candidate package

A distributor or integrator prepares `hiroute`, `hirouted`, and the pinned CPA from one exact committed candidate, then
builds a reproducible archive and companion manifest:

```sh
python3 scripts/collect-third-party-licenses.py \
  --cargo-target x86_64-unknown-linux-gnu \
  --cpa-source-repo /path/to/CLIProxyAPI \
  --output /absolute/output/notices

python3 scripts/package-standalone.py build \
  --version 0.2.0 --revision FULL_COMMIT_SHA \
  --target x86_64-unknown-linux-gnu \
  --hiroute /absolute/path/hiroute \
  --hirouted /absolute/path/hirouted \
  --cpa-binary /absolute/path/cliproxyapi \
  --cpa-version PINNED_VERSION \
  --cpa-license /absolute/path/CLIProxyAPI-LICENSE \
  --notices /absolute/output/notices \
  --output /absolute/output

python3 scripts/install-standalone.py install \
  --manifest /absolute/output/PACKAGE.tar.gz.json \
  --archive /absolute/output/PACKAGE.tar.gz
```

After distribution integration, the installer also accepts
`--manifest-url https://.../PACKAGE.tar.gz.json`. Redirects and archives must remain HTTPS.
Before any write, the installer checks the platform, archive SHA-256, closed file set, and
per-file digests. Never infer an unpublished download URL from this page.

The collector uses only locked Rust dependencies and the CPA commit pinned by
`vendor/cpa/source.json`. `--cpa-source-repo` supplies local objects for that exact commit and
does not use the checkout's current branch. The packager requires the collector's closed
output and installs licenses and deterministic manifests for HiRoute, CPA, and transitive
dependencies under `licenses/`. Missing material fails packaging.

Installation writes only current-user paths:

- stable entry points: `$HOME/.local/bin/hiroute` and `hirouted`, with no `sudo`;
- versioned binaries: `$HOME/.local/lib/hiroute/<version>`;
- read-only resources and marker: `$HOME/.local/share/hiroute`;
- management Skill: `$HOME/.agents/skills/hiroute-management` and
  `$HOME/.claude/skills/hiroute-management`;
- Linux state: `${XDG_STATE_HOME:-$HOME/.local/state}/hiroute`.

The installer places the Skill in private, current-user-owned directories. A symlink,
non-directory, foreign-owned, or group/world-writable Skill parent aborts before any write.
The same install command handles first install, repair, and upgrade. After an update, run
`hiroute service restart --output json` explicitly to move the live daemon to the new version.

Installation fails if `/Applications/HiRoute.app`, `~/Applications/HiRoute.app`, or an active
Desktop Local Control exists. Stop and uninstall standalone before switching in the other
direction.

## Start and verify real readiness

```sh
hiroute service start --output json
hiroute service status --output json
hiroute system status --output json
hiroute gateway show --output json
```

Business readiness requires all of these facts:

- `service status` reports `data.local_control_ready=true`;
- `system status` reports `data.daemon=role_all` and `data.gateway=ready`;
- `gateway show` reports `data.ready=true` and an actual `connect_address`.

A successful process-manager command is not service readiness. If a systemd user unit is
unavailable, run the service in a managed foreground terminal:

```sh
hiroute service run
```

Other lifecycle commands:

```sh
hiroute service doctor --output json
hiroute service logs --output json
hiroute service restart --output json
hiroute service stop --output json
hiroute service autostart enable --output json
hiroute service autostart disable --output json
```

Login autostart and starting the current session are separate actions. `SIGTERM` and `SIGINT`
request an orderly shutdown.

### Subscription network proxy

Enable your proxy in the terminal before `hiroute service start`, `restart`, or `run`.
These commands capture `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY` and their lowercase
forms for managed subscription requests, including model discovery. HTTPS targets
use `HTTPS_PROXY`; `NO_PROXY` excludes matching targets. `ALL_PROXY`, PAC and OS
network settings are not read. HiRoute does not execute shell startup files.

The CLI saves the six-variable snapshot in an owner-only file at
`~/.local/share/hiroute/service/proxy-environment.json`. This lets a separately
started systemd/launchd service use the terminal's configuration without modifying
other services. Only CPA consumes it; local management stays direct. Login autostart
reuses the saved snapshot. To change proxies, update your shell and run
`hiroute service restart`; `start` on an already-running service leaves it unchanged.
To clear a saved proxy, unset all six variables and restart. Proxy failure does not
fall back to direct connections. Do not share the private snapshot: proxy URLs may
contain credentials. This does not change native Codex login or token refresh ownership.

## Discover the current public contract first

The CLI has two public-contract layers:

- Host management commands are defined by root and family help. `service`, `gateway`, and
  `protected-input` intentionally do not appear in the Application release manifest.
- Application/Local Control commands are defined by Released descriptors from
  `schema list/show` plus complete leaf `--help`.

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
hiroute schema list --output json
hiroute schema show --command-id compute.connection.test --output json
hiroute compute connection test --help
```

Do not use presence in `schema list` to decide whether a Host command exists. Use only listed
Application/Local Control commands. CLI and standalone daemon both reject Planned business
commands; a handler or development-build path does not make a command public. Every machine
response is a `hiroute.machine-envelope/v2`. Automation reads `status`, `data`, stable
`error.code`, `warnings[].code`, and `next_actions[].command_id`; it never parses display text.

The JSON examples below are ordinary credential-free files. `jq` is used only to demonstrate
field extraction and is not a HiRoute dependency.

## Connect a Native API model source

Credentials can be registered only through a protected file descriptor. Never put a key in
argv, environment variables, ordinary JSON, logs, or an Agent conversation:

```sh
chmod 600 /absolute/private/provider-key
exec 3</absolute/private/provider-key
hiroute protected-input register \
  --candidate candidate/native/my-provider \
  --secret-fd 3 --output json
exec 3<&-
```

Construct a bounded check from the `compute.connection.test` schema. This complete shape uses
Responses API, Bearer authentication, and one manually declared text model for a provider
without a `/models` catalog. Replace endpoint, model, and capability facts without adding a
secret field:

```json
{
  "kind": "native",
  "request": {
    "draft": {
      "inference_model_id": null,
      "candidate_ref": "candidate/native/my-provider",
      "lineage_ref": "lineage/native/my-provider",
      "display_name": "My Responses API",
      "existing_source_id": null,
      "edit_revision": 1,
      "check_id": "check/native/my-provider-1",
      "base_url": "https://provider.example/v1",
      "base_kind": "api_root",
      "request_path_override": null,
      "inventory_path_override": "/v1/models",
      "protocol": "responses",
      "protocol_profile_id": "profile/custom/responses",
      "protocol_profile_revision": 1,
      "authentication": {"kind": "bearer"},
      "configuration_revision": 1,
      "models": [{
        "upstream_model_id": "provider-model-id",
        "display_name": "Provider model",
        "catalog_configuration_id": null,
        "membership": "user_declared",
        "capabilities": {
          "tool": {"value": true, "basis": "user_declared"},
          "vision": {"value": false, "basis": "user_declared"},
          "streaming": {"value": true, "basis": "user_declared"},
          "context_tokens": {"value": 32768, "basis": "user_declared"},
          "max_output_tokens": {"value": 4096, "basis": "user_declared"},
          "native_reasoning": {
            "value": {"kind": "fixed", "profile": "provider-default"},
            "basis": "user_declared"
          }
        }
      }]
    },
    "input_candidate": {
      "candidate_ref": "candidate/native/my-provider",
      "candidate_revision": 1
    }
  }
}
```

```sh
hiroute compute connection test --request-stdin --output json \
  < native-test.json > native-checked.json
hiroute compute list --output json > compute-before-save.json
```

Choose a `model_ref` with `selectable=true` from
`native-checked.json.data.candidate.models[]`. Copy current revisions from
`compute-before-save.json.data.revisions` and construct the save change:

```json
{
  "change": {
    "schema": "hiroute.compute-management-change/v2",
    "subject": {"kind": "candidate", "candidate": {
      "candidate_ref": "CANDIDATE_REF_FROM_CHECK",
      "candidate_revision": 1
    }},
    "expected_revisions": {"target": 0, "dependencies": {}},
    "selected_model_refs": ["SELECTABLE_MODEL_REF"],
    "intent": "save_ready",
    "key_edits": []
  }
}
```

Preview first. Pass the returned `spec`, `accept_digest`, `expected_revisions`, and a fresh
idempotency key unchanged to apply:

```sh
hiroute compute connection preview --request-stdin --output json \
  < compute-preview-request.json > compute-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"save-my-provider-1"}' \
  compute-preview.json \
  | hiroute compute connection apply --request-stdin --output json \
  > compute-apply.json

hiroute operations get "$(jq -r '.operation.operation_id' compute-apply.json)" --output json
hiroute compute list --output json
hiroute compute show "SOURCE_ID" --output json
hiroute protected-input release \
  --candidate candidate/native/my-provider --output json
```

Do not replace the idempotency key after an uncertain apply response. Keep the original
request and query the same scope through the public recovery entry:

```json
{
  "principal_kind": "interactive_user",
  "operation_kind": "ApplyComputeSave",
  "idempotency_key": "save-my-provider-1",
  "accepted_digest": "ACCEPT_DIGEST_FROM_PREVIEW"
}
```

```sh
hiroute operations find --request-stdin --output json < operation-find.json
```

The same key and content returns the original Operation. The same key with different content
is rejected deterministically and never writes twice.

## Discover and save a subscription source

```sh
hiroute compute connection options --output json > connection-options.json
```

`data.subscriptions` reports discovery state and candidates exactly. Select one
`connector_owned` candidate, wrap it as `{"candidate": ...}`, and start its check with the
same Preview/Apply rules:

```sh
jq '{candidate:([.data.subscriptions.candidates[] |
    select(.provenance=="connector_owned")] | first)}' \
  connection-options.json \
  | hiroute compute connection preview --request-stdin --output json \
  > subscription-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"check-subscription-1"}' \
  subscription-preview.json \
  | hiroute compute connection apply --request-stdin --output json \
  > subscription-operation.json

jq '{action:"result",operation:.operation}' subscription-operation.json \
  | hiroute compute connection authorize --request-stdin --output json \
  > subscription-checked.json
```

Only `data.status=verified` means the check passed. Save `data.checked_candidate` and
`data.validation` through the same compute-save Preview/Apply flow above. Discovery,
authorization check, saving, and a real model call are different facts; never describe a
discovered source as already saved or called.

## Create, update, and publish routing

Query real candidates instead of hand-writing an internal binding:

```sh
printf '{}\n' | hiroute routing options --request-stdin --output json > routing-options.json
```

Select a `binding_id` from `data.candidates[]` and construct a complete editor. A fixed single
model is a valid minimal plan:

```json
{
  "change": {
    "schema": "hiroute.plan-content-change/v2",
    "target": {"intent": "create", "creation_key": "my-first-route"},
    "editor": {
      "schema": "hiroute.plan-editor/v2",
      "display_name": "Daily coding",
      "purpose": "Coding requests from my local Agents",
      "mode": "fixed_model",
      "candidates": [{"binding_id": "BINDING_FROM_ROUTING_OPTIONS"}],
      "smart": {
        "economy": [], "primary": [],
        "judgment": {
          "degree": {
            "simple_threshold_millis": 800,
            "instructions": "Judge the reasoning and uncertainty required by the current task.",
            "simple": "Explicit, bounded work that follows established patterns.",
            "complex": "Work requiring investigation or materially different design choices."
          },
          "competence": {
            "floor_millis": 500,
            "instructions": "Assess useful progress in the completed stage identified by the assessment target.",
            "criteria": ["No useful progress or substantial correction needed.", "Useful but incomplete or uneven progress.", "Reliable progress with no material correction."]
          }
        },
        "reselect_on_user_message": false,
        "classifier": {"kind": "local_rules"}, "complex_keywords": []
      },
      "free": {"candidates": [], "primary": [], "primary_fallback": false},
      "delegation_enabled": false,
      "requirements": {},
      "limits": {
        "maximum_attempts": 1,
        "request_timeout_ms": 30000,
        "attempt_timeout_ms": 30000
      }
    },
    "consumed_draft": null
  }
}
```

```sh
hiroute routing preview --request-stdin --output json \
  < route-change.json > route-preview.json

jq --slurpfile change route-change.json \
  '{change:$change[0].change,accept_digest:.data.change_digest,
    expected_revisions:.data.expected_revisions,
    idempotency_key:"publish-my-first-route-1"}' \
  route-preview.json \
  | hiroute routing apply --request-stdin --output json > route-apply.json

hiroute routing list --output json
hiroute routing show PLAN_ID --output json
```

For an update, set `target` to
`{"intent":"update","plan_id":"PLAN_ID","expected_head_revision":CURRENT_HEAD}` and
submit the complete editor. A stale revision/digest returns conflict while the old publication
continues serving. Read again, edit, Preview, and Apply. A normal model request performs only
routing and never starts a Worker implicitly because the plan enables delegation.

## Connect, check, and restore Codex

```sh
hiroute agents scan --output json > agents.json
hiroute agents check agent_codex_default \
  --scope native-authentication --output json
```

`configuration`, `native-authentication`, and `collaboration` are bounded same-UID local
checks. They use no second authorization token and make no upstream model call. `live` does
make a real model request and still requires the command help's explicit consent and a
protected probe grant.

Copy the Codex `context_id` from scan results and connect it to a published `PLAN_ID`:

```json
{
  "spec": {
    "schema_version": {"major": 2, "minor": 0},
    "context_id": "CONTEXT_ID_FROM_SCAN",
    "model": {
      "intent": "configure",
      "settings": {
        "mode": "codex_default",
        "native_model_mode": "hiroute_only",
        "fixed_models": [],
        "allowed_plan_ids": ["PLAN_ID"],
        "default_selection": {"kind": "plan", "plan_id": "PLAN_ID"}
      }
    }
  }
}
```

```sh
hiroute agents connect preview --request-stdin --output json \
  < agent-connect.json > agent-preview.json

jq '{spec:.data.spec,accept_digest:.data.accept_digest,
     dependency_digest:.data.dependency_digest,
     expected_revisions:.data.expected_revisions,
     idempotency_key:"connect-codex-1"}' \
  agent-preview.json \
  | hiroute agents connect apply --request-stdin --output json > agent-apply.json

hiroute agents connect status CONTEXT_ID --output json > agent-status.json
```

The standalone daemon is already a resident service, so Preview does not require a Desktop
login item. HiRoute changes only the Codex fields that it owns. Continue using the original
`codex` entry afterward; its model requests enter the local Gateway. Save
`agent-status.json.data.restore_point_ref`. To restore:

```json
{
  "spec": {
    "schema_version": {"major": 2, "minor": 0},
    "context_id": "CONTEXT_ID",
    "model": {"intent": "restore", "restore_point_ref": "RESTORE_POINT_REF"}
  }
}
```

Pass this spec through `agents restore preview`, copy the returned Preview fields to
`agents restore apply`, and finish with `agents connect status`. Restore removes only fields
still owned by HiRoute; concurrent user changes conflict instead of being overwritten. Claude
Code uses its public profile/launcher contract. Do not configure a Responses-only Codex plan
for a surface that supports only Messages.

## Sessions, actual selection, and usage facts

After an Agent or Worker request completes:

```sh
hiroute sessions list --include-unlinked --limit 50 --output json
hiroute sessions show SESSION_ID --output json
hiroute sessions receipt RECEIPT_ID --output json
hiroute sessions status --output json
hiroute value show --routing PLAN_ID --session SESSION_ID --output json
hiroute observation plan-quality samples --plan-id PLAN_ID --output json
hiroute observation plan-quality samples --session-id SESSION_ID --limit 50 --output json
```

`sessions show` returns facts and timeline by default, not conversation bodies. Ordered
RoutingReceipt facts such as `route_decision`, `attempt_started`, and `usage_and_cache` record
the actual plan, model choice, Attempt, and known upstream-reported tokens. `value show`
returns only existing ledger value. Without trustworthy price evidence, an amount stays
`null`; unknown amounts or tokens without value-ledger rows are never fabricated as zero
cost. Bodies, search, catalog, and ancestry each require their own precise protected
capability.

`observation plan-quality samples` returns the same stage facts used by the plan's
competence view. Each sample's `branch_execution` identifies its actual model group,
candidate position and frozen judgment policy. Missing or partial scores remain distinct
from zero; a stage score does not rewrite its opening selection reason. Use the returned
cursor to read further pages. This facts-only query does not call a model or grant access
to conversation bodies; discover its filters through the command's `--help` or `schema show`.

## Configure a Worker and delegate a task

Worker reuses the existing commands. Discovery does not install or select software:

```sh
hiroute worker dependencies discover --harness codex_cli --output json \
  > worker-discovery.json
```

Copy `selection_revisions[].revision` for the same Harness and select complete absolute paths
whose state is `found`:

```json
{
  "harness": "codex_cli",
  "adapter_path": "/absolute/path/to/codex-acp",
  "cli_path": "/absolute/path/to/codex",
  "node_path": "/absolute/path/to/node",
  "expected_selection_revision": 0
}
```

```sh
hiroute worker dependencies select --request-stdin --output json \
  < worker-selection.json
hiroute worker executors --output json
hiroute worker plans --output json
```

Set routing-editor `delegation_enabled` to `true`, add
`"work":{"harness":"codex_cli","protocol":"responses"}`, and publish again. Then submit,
locate, wait for, and read the task:

```sh
hiroute worker exec --plan PLAN_ID --cwd /absolute/project \
  --no-wait --submission-key task-20260921-1 --file task.txt --output json

hiroute worker status \
  --submission task-20260921-1 --operation start --output json
hiroute worker status --run RUN_ID --output json
hiroute worker wait --run RUN_ID --wait-timeout 30 --output json
hiroute worker result --run RUN_ID --output json
hiroute worker list --output json
```

A wait timeout means only that work is still running and does not cancel it. After an
uncertain response, query with the original submission key or replay that exact JSON unchanged
through the same `worker exec`; never replace the key. Duplicate submission returns the same
run and does not execute twice. `selection_revision` is the concurrency revision for Worker
installation selection, not a Codex, Claude, or HiRoute CLI/package version.

## Gateway listener

```sh
hiroute gateway show --output json
hiroute gateway set --address 127.0.0.1 --port auto --output json
hiroute gateway set --address 192.0.2.10 --port 8317 \
  --accept-remote-risk --output json
hiroute gateway recover --output json
```

`set` saves desired state, restarts, and advances applied state only after real readiness.
The first automatically chosen port is persisted. Local management clients see a
`0.0.0.0` listener as `127.0.0.1:<port>`. A non-loopback address requires explicit risk
acceptance. HiRoute never changes a firewall, configures TLS, or manages a remote Agent.
IPv6, multiple listeners, and firewall management are not supported.

## Common errors and recovery

- `DAEMON_UNAVAILABLE`: on Standalone, run `hiroute service status`, then
  `hiroute service start` if needed, and diagnose with `service doctor` and `service logs`.
  On Desktop, launch or restore the application. For an isolated instance, confirm the same
  `HOME`, `XDG_STATE_HOME`, and `XDG_RUNTIME_DIR`. The CLI does not start a service or replay
  a request automatically.
- `UNKNOWN_COMMAND`: for Host management, read root and family `--help`. If an Application
  command is absent from the current release manifest, do not try an internal operation name
  or staged path; upgrade or use an entry listed by `schema list`.
- `INVALID_ARGUMENTS`: read leaf `--help` and `schema show`. Strict schemas reject unknown
  fields.
- `REVISION_CONFLICT` / `CHANGE_PREVIEW_STALE`: read current resources/options and Preview
  again. Never modify and force an old Preview through Apply.
- `IDEMPOTENCY_KEY_REUSED`: the key already names another digest. Use `operations find` to
  inspect the winning Operation; do not claim that new content was applied.
- Lost Apply response: keep the original spec/digest/revisions/key and call
  `operations find` first. An explicit retry of the same content uses the original key.
- Agent restore conflict: the user or Agent changed a managed field. Stop automatic overwrite,
  read status, and confirm again.
- Successful model request with unknown cost: inspect the RoutingReceipt's actual model and
  usage. `null` without price evidence is correct and does not mean free.

## Management Skill

The package installs the same `hiroute-management` Skill in the generic Agent and Claude Code
Skill directories. It organizes only the public commands on this page: service checks,
source/routing configuration, Agent integration/restoration, observation queries, and Worker
operations. It implements no business validation, directly edits no database or Agent
configuration, handles no plaintext credential, and never installs, publishes, changes a
listener, cancels a task, or enables autostart on its own. For Host commands, an Agent first
reads `hiroute --help` and the family `--help`. For Application/Local Control commands, it
first reads `schema list/show` and complete leaf `--help`, then stays within the user's
explicitly authorized external side effects.

## Uninstall

Stop the service first, then run:

```sh
python3 scripts/install-standalone.py uninstall
```

Uninstall removes only marker-recorded stable entries, the current version's programs,
service definition, and the two Skills while they are still HiRoute-owned. Any externally
replaced entry aborts instead of being overwritten. Business storage, diagnostics, and
sessions are retained by default. Deleting retained data requires a separate confirmation of
the exact directory and that its contents are no longer needed.

## Decision services and branch routing

Desktop manages these connections under **Models → Decision models**; the Released CLI keeps
`decision services list/apply/test`. Decision models and custom extensions are independent of
general model sources that execute tasks. Discover the installed contract first:

```sh
hiroute decision services list --output json
hiroute schema show --command-id decision.services.apply --output json
hiroute decision services apply --help
hiroute decision services test --help
```

There is no separate `decision services preview` command. The first `apply` previews without
saving. Put the complete request below in `decision-preview-request.json`. The outer request
contains `schema_version` and `spec`; the inner ChangeSpec has its own `schema_version`:

```json
{
  "schema_version": {"major": 1, "minor": 0},
  "spec": {
    "schema_version": {"major": 1, "minor": 0},
    "command_id": "decision.services.apply",
    "resource_id": "decision-main",
    "desired_state": {
      "id": "decision-main",
      "expected_revision": 0,
      "service": {
        "id": "decision-main",
        "revision": 1,
        "name": "Bailian decision",
        "connection": {
          "kind": "system_one",
          "provider": "bailian-token-plan",
          "model": "decision-model-preview",
          "endpoint": "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone",
          "timeout_ms": 10000,
          "auth_header": {"name": "Authorization", "value_secret_ref": "decision/main/r1"}
        }
      },
      "input_slot": "candidate/decision-main"
    }
  }
}
```

Before preview, register `candidate/decision-main` using protected input. The private file
must contain the complete `Bearer ...` header value for this example, not a bare API key:

```sh
chmod 600 /absolute/private/decision-header
exec 3</absolute/private/decision-header
hiroute protected-input register \
  --candidate candidate/decision-main --secret-fd 3 --output json
exec 3<&-

hiroute decision services apply --request-stdin --output json \
  < decision-preview-request.json > decision-preview.json
```

Proceed only when the preview has no blockers. Save the exact normalized spec, digest and
expected revisions returned by the preview in an apply request, then submit it:

```sh
jq '{schema_version:{major:1,minor:0},spec:.data.normalized_spec,
     accept_digest:.data.change_digest,expected_revisions:.data.expected_revisions,
     idempotency_key:"save-decision-main-1"}' \
  decision-preview.json > decision-apply-request.json

hiroute decision services apply --request-stdin --output json \
  < decision-apply-request.json > decision-apply.json

hiroute decision services list --output json > decision-saved.json
hiroute protected-input release --candidate candidate/decision-main --output json
```

Check the returned Operation state in `decision-apply.json`. If delivery is uncertain, use
`operations find/get` in the original idempotency domain and retain the exact apply request;
do not replace its key. An edit expects the current revision and saves the next revision
(for example, `expected_revision: 1`, service `revision: 2`). Deletion uses `service: null`
and the current expected revision. Connections referenced by routes, drafts or retained
historical versions cannot be deleted. Credential rotation uses a new protected reference
so previously published versions retain their own credential.

Test the exact saved r1, rather than an unsaved draft or whichever version happens to be latest:

```sh
jq -e --arg id decision-main --argjson revision 1 \
  '[.data.services[] | select(.id==$id and .revision==$revision)] |
   if length == 1 then
     {schema:"hiroute.classifier-decision-test/v1",
      classifier:{kind:"decision_service",service:.[0]}}
   else error("Expected saved connection revision is not in this list") end' \
  decision-saved.json > decision-test-request.json

hiroute decision services test --request-stdin --output json < decision-test-request.json
```

The fixed synthetic test may consume quota. Check `data.outcome` and `data.failure_code`,
not only the CLI exit code. It tests that saved connection's transport and required response
fields, reads no real conversation, runs no execution model and produces no competence sample.
A pass is not a task-quality or continuing-health guarantee. Saving a connection and testing
it are separate actions; neither publishes a route.

Plans use the existing `routing preview/apply` transaction with `hiroute.plan-editor/v2`:

| Mode | Editor fields |
| --- | --- |
| `smart_saving` | `smart.classifier`, `smart.economy`, `smart.primary` and `smart.judgment` |
| `custom_branches` | `branch_routing.classifier`, `branch_routing.branches`, `branch_routing.default_branch_id`, `branch_routing.judgment` and `branch_routing.reselect_on_user_message` |

`classifier` is `{kind: "decision_service", service: <complete saved connection revision>}`
for a decision model or custom extension. Only Smart saving may instead use
`{kind: "local_rules"}`. Publication verifies the connection ID, revision and full content
against its saved immutable version; there is no inline `rest` connection.

A judgment has `degree: {simple_threshold_millis, instructions, simple, complex}` and
`competence: {floor_millis, instructions, criteria}`. Thresholds use integers from 0 to 1000:
800 means 0.8 and 500 means 0.5. `criteria` contains exactly three nonempty descriptions for
0, 0.5 and 1. The short prompts in the fixed-route example above are illustrative editable
criteria, not a claim to reproduce the complete built-in defaults.

Each custom branch has `id`, `name`, `condition`, regular `candidates` and `primary_candidates`
(an empty array for a single-group branch). Optional branch `judgment` is a complete override;
omit it or set it to `null` to follow plan defaults. Customize by copying the whole effective
judgment, then editing it; restore by removing that whole override. Task `condition` remains
separate from degree and competence prompts. A branch without primary skips degree judgment
while competence can still be observed.

Publish pins the connection revision, task conditions, judgment policies, ordered candidates
and reasoning configuration. Saving r2 leaves a published r1 route unchanged until it is
selected and republished; saved historical r1 remains publishable even if `list` now returns r2.
Every new user turn chooses again. Tool continuations and replay inherit the frozen choice
only when the same turn is identifiable, history continues, and the decision remains reusable.
Compaction or history reconstruction that breaks continuity or prevents reuse causes a fresh decision.
Only this call's complete compatible same-category low score can trigger primary protection;
missing/partial scores are not zero. Regular candidates relay to primary within the same branch,
while a direct primary selection stays in primary.

Custom connections use `connection.kind: "custom"` and implement HiRoute's
[general Decision API](../decision-extensions/api/README.md). Built-in connections use the
vendor [System One mapping](../decision-extensions/api/system-one-design.md). See the
[decision protocol](../decision-extensions/api/decision-design.md) for selection and
failure rules. Tool selection has no runtime entry in this release.
