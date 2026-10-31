# HiRoute Linux headless CLI

[简体中文](standalone-cli.zh-CN.md)

Standalone is the single-user mode that does not install Desktop. The production `hiroute`
CLI, role-all `hirouted` daemon, Local Control, Gateway, business storage, and observation
form one control and execution loop. It uses the local control socket's OS access permissions, needs no
second “CLI management token,” and exposes no second Agent-management API.

Linux standalone and macOS HiRoute Desktop must not manage the same user installation
at the same time. The Linux package provides neither a system-wide multi-user install
nor runtime downloads or automatic updates. Windows has a separate
[current-user Desktop installer](windows-desktop-installation.md) that shares its
headless installation. macOS can use the same candidate-package mechanism, but this
page and the Quickstart are validated for Linux.

## Installation

The installed daemon also provides the [Web console](web-console.md). It defaults
to off on Linux. Configure its scope and port with installer options or
`hiroute console init`, then start/restart the service. `hiroute console open`
returns a one-use URL for the actual local or remote listener without starting
listening. Remote HTTP also accepts owner-created CLI access tokens and displays
plaintext warnings. It does not change the Gateway listener.

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
  --version RELEASE_VERSION --revision FULL_COMMIT_SHA \
  --target x86_64-unknown-linux-gnu \
  --hiroute /absolute/path/hiroute \
  --hirouted /absolute/path/hirouted \
  --cpa-binary /absolute/path/cliproxyapi \
  --cpa-version PINNED_VERSION \
  --cpa-license /absolute/path/CLIProxyAPI-LICENSE \
  --notices /absolute/output/notices \
  --console-assets /absolute/path/apps/desktop/dist \
  --output /absolute/output

python3 scripts/install-standalone.py install \
  --manifest /absolute/output/PACKAGE.tar.gz.json \
  --archive /absolute/output/PACKAGE.tar.gz
```

Use the component version and full source revision of the same committed candidate for
`RELEASE_VERSION` and `FULL_COMMIT_SHA`.

After distribution integration, the installer also accepts
`--manifest-url https://.../PACKAGE.tar.gz.json`. Redirects and archives must remain HTTPS.
Before changing the installation, the installer checks the platform, archive SHA-256, closed file set, and
per-file digests. Never infer an unpublished download URL from this page.

HTTPS archive downloads use bounded requests and retries when a transfer ends early.
When combining byte ranges, the installer requires one unchanged strong object identity;
servers without range support retain whole-file downloads. All size and digest checks
still complete before changing an existing installation.

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

## Storage upgrade recovery

Before replacing an installed program, the verified candidate performs read-only
preflight. When a history-format cutover is required, the installer retains the
complete original program/resources and installation records in private service
state; the daemon receives that handoff directly, including under systemd.
Current-format repair skips this archival copy and its extra space reserve.
Repeating the exact same-version installed daemon also preserves existing handoffs
without requiring a newer preflight helper from an older package; normal storage
admission still runs when the service starts. Failed
preparation removes only its unpublished copy; once the handoff may have been
consumed, its material is retained even after later installation rollback.
Do not delete `upgrade-packages`, `upgrade-package.json`, sealed history or
`storage.upgrade-backups` to make room for an upgrade. Budget their disk use.

If startup fails, use `hiroute service recovery --output json` even while Local
Control is unavailable. It reports the storage-owned cause and action plus the
startup diagnostic location. Missing packages do not authorize program-only
downgrade. Preserve data, keys, programs and journals; follow diagnostic/support
steps when necessary material is unavailable. A resumable batch uses the current
program. Whole-group recovery uses that batch's `RECOVERY.md`, `恢复说明.md` and,
when present, `PROGRAM-LOCATIONS.md`: stop the service and automatic restarts,
preserve newer data/program separately, restore all stores/keys/Observation and
recorded auxiliary files, then restore the matching program/resources and service
registration at their recorded locations. Reload the manager and verify actual
Control/Gateway readiness and a real request. Installation-command success alone
is insufficient.

Explicit headless operators may still provide the absolute complete old-package
directory via `HIROUTE_RECOVERY_PACKAGE` on the daemon's first sealing startup.
For systemd, first stop the failed service with `hiroute service stop --output json`.
Run `systemctl --user edit ai.hiroute.cli.service` and add a temporary override:

```ini
[Service]
Environment="HIROUTE_RECOVERY_PACKAGE=/absolute/path/to/verified-complete-old-package"
```

Use the actual retained package path, then run `systemctl --user daemon-reload`
and `hiroute service start --output json`. Inspect `hiroute service recovery
--output text` if startup still refuses. After successful cutover, remove only
this temporary `Environment` entry, reload the manager, restart the service and
verify actual readiness and a request. Preserve unrelated overrides; do not use
`systemctl revert` to discard them.

Set it on the foreground daemon's actual invocation when running without a manager. Exporting a variable in a shell and calling systemctl
does not deliver it to the managed daemon. Ordinary installer upgrades use their
protected handoff instead. Missing keys, unverified sources and unknown phases
require preservation and diagnosis/support; repeated restarts cannot recreate them.
The exact unchanged-program repair path does not require a new preflight protocol;
changing program bytes requires a candidate that implements the supported checks.

A format change preserves configuration, credentials and routing and seals old
HiRoute observations. Current history/statistics start at cutover; native Agent
history is unchanged. New current-format records survive ordinary restarts.

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

## Discover the current public contract first

The CLI has two public-contract layers:

- Host management commands are defined by root and family help. `service`, `gateway`, and
  `protected-input` and `subscription-proxy` intentionally do not appear in the Application release manifest.
- Application/Local Control commands are defined by Released descriptors from
  `schema list/show` plus complete leaf `--help`.

```sh
hiroute --help
hiroute service --help
hiroute gateway --help
hiroute protected-input --help
hiroute subscription-proxy --help
hiroute schema list --output json
hiroute schema show --command-id compute.connection.test --include-schema --output json
hiroute compute connection test --help
```

Do not use presence in `schema list` to decide whether a Host command exists. Use only listed
Application/Local Control commands. CLI and standalone daemon both reject Planned business
commands; a handler or development-build path does not make a command public. Every machine
response is a `hiroute.machine-envelope/v2`. Automation reads `status`, `data`, stable
`error.code`, `warnings[].code`, and `next_actions[].command_id`; it never parses display text.

The JSON examples below are ordinary credential-free files. `jq` is used only to demonstrate
field extraction and is not a HiRoute dependency.

## Test a catalog API connection

`compute connection test` requires a JSON input source and a nested `{kind, request}`
body. Use `registered` for an offered catalog option such as OpenRouter; use `native`
with `request.draft` for a custom API. `--non-interactive` does not supply a request.
The installed full schema includes both templates in `data.schemas.examples` and
explains their prerequisites in `data.schemas.example_guidance`. Full schema/template
discovery requires `--include-schema`; v0.3.1 returns descriptors only.

With the service ready, Python 3 installed and a private mode-0600 file containing
only your OpenRouter API key, replace `KEY_FILE` below. The script obtains the exact
catalog from the running service and builds the registered template. The current
bundled OpenRouter option has no remote directory probe. With `inference_model_id=null`,
the check sends no request to OpenRouter: it prepares a candidate from bundled facts
without verifying connectivity or the API key. It does not save a source.
It attempts to release its protected input on exit and retains separate result/error
files for diagnosis. Only references enter ordinary JSON; the key stays on FD 3.

```sh
(
set -eu
umask 077
KEY_FILE='/absolute/private/openrouter-key'
work=$(mktemp -d "${TMPDIR:-/tmp}/hiroute-connection.XXXXXXXX")
candidate="candidate/native/openrouter-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
hiroute schema show --command-id compute.connection.test --include-schema --output json > "$work/contract.json"
hiroute compute connection options --output json > "$work/options.json"
hiroute protected-input register --candidate "$candidate" --secret-fd 3 --output json \
  3< "$KEY_FILE" > "$work/input.json"
trap 'hiroute protected-input release --candidate "$candidate" --output json >&2 || echo "Retry releasing this candidate if cleanup failed: $candidate" >&2' EXIT
python3 - "$work" "$candidate" > "$work/registered-request.json" <<'PY'
import json, pathlib, sys
work, candidate = pathlib.Path(sys.argv[1]), sys.argv[2]
contract = json.loads((work / 'contract.json').read_text())
options = json.loads((work / 'options.json').read_text())
registration = json.loads((work / 'input.json').read_text())
if any(x.get('status') != 'succeeded' for x in (contract, options, registration)):
    raise SystemExit('Read contract.json, options.json and input.json for the failed prerequisite')
if registration.get('data', {}).get('registered') is not True:
    raise SystemExit('Protected input was not registered')
option_id = 'openrouter.free.global.v1'
option = next((x for x in options['data']['options']
               if x['connection_option_id'] == option_id), None)
if option is None or not option.get('registered_check_available', False):
    raise SystemExit('The current catalog does not offer this registered check')
body = next(x for x in contract['data']['schemas']['examples'] if x['kind'] == 'registered')
request = body['request']
request.update(connection_option_id=option_id, expected_catalog=options['data']['catalog'],
               candidate_ref=candidate, lineage_ref='lineage/native/' + candidate.rsplit('/', 1)[1],
               check_id='check/native/' + candidate.rsplit('/', 1)[1], edit_revision=1,
               input_candidate={'candidate_ref': candidate, 'candidate_revision': 1},
               models=[], inference_model_id=None)
print(json.dumps(body, indent=2))
PY
set +e
hiroute compute connection test --request-stdin --output json \
  < "$work/registered-request.json" > "$work/result.json" 2> "$work/stderr.txt"
rc=$?
set -e
cat "$work/result.json"
cat "$work/stderr.txt" >&2
if [ "$rc" -eq 0 ]; then
python3 - "$work/result.json" <<'PY'
import json, sys
check = json.load(open(sys.argv[1]))['data']
fields = ('reachability', 'authentication', 'directory', 'inference')
print('Remote check results: ' + ', '.join(name + '=' + check[name] for name in fields))
if check['reachability'] == 'not_run' and check['authentication'] == 'unknown':
    print('Remote connectivity and API key were not verified.')
PY
fi
printf 'connection-test exit code: %s; result files: %s\n' "$rc" "$work"
exit "$rc"
)
```

Read `data.reachability`, `data.authentication`, `data.directory` and `data.inference`
in the result. For the current bundled OpenRouter option this recipe reports
`reachability=not_run`, `authentication=unknown`, `directory=not_run` and
`inference=not_run`. Exit code 0 / `status=succeeded` means the command completed;
it does not establish provider verification. Catalog models or a selectable candidate
do not establish it either. A remote check requires an explicitly selected supported
probe; selecting an inference model can consume quota.

The protected-input registration protocol uses `candidate_revision=1` for a newly
registered credential. This is distinct from revisions returned by a completed model
check. Keep `expected_catalog` as the whole current `data.catalog` object. A catalog
mismatch requires fetching current options and rebuilding the check; template digests
are placeholders. The same request can be sent with
`--request-file "$work/registered-request.json"` instead of stdin redirection.
For custom APIs, save the `native` template as `native-request.json` and supply the
actual endpoint/protocol facts and protected-input reference; the next section shows
a complete manually declared-model request.

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

## Rename, append or remove saved connections

Use the same `compute connection preview` / `apply` sequence with the optional
`change.edit` field. Omit `edit` to keep the existing save behavior. Read current
`compute list` revisions before preview; pass its returned request unchanged to apply.

| `edit` | Subject | `selected_model_refs` |
| --- | --- | --- |
| `{"action":"rename","display_name":"Team API"}` | Saved source | Empty |
| `{"action":"append_models"}` | Freshly checked candidate for the existing source | Models to add |
| `{"action":"remove_models"}` | Saved source | Models to remove |
| `{"action":"delete"}` | Saved source | Empty |

A saved-source subject is `{"kind":"saved_source","source_id":"SOURCE_ID"}`.
Keep `key_edits` empty for these saved-source edits and append. Names must be distinct,
nonempty and at most 60 characters. A new candidate save can also carry the rename
edit to name its independent connection; its normal protected key input is still required.
Each independently created connection owns its endpoints and credentials, even when it
starts from the same provider template or uses the same literal key.

Append preserves existing model records, bindings, credentials and enabled state.
Removing the last model requires explicit `delete`; deleting a connection also cleans
its HiRoute-owned credentials, while preserving subscription sign-ins and usage history.
`affected_plan_refs` lists blockers in the preview: disabled routes, saved drafts,
and retained execution or recovery versions also prevent removal. Apply rechecks
references, so a new reference after preview can still reject the operation.

After deleting a route or moving its models, the previous publication may still retain
references. Refresh them through the normal routing publication transaction:

```json
{"change":{"schema":"hiroute.publication-checkpoint-change/v1"}}
```

Pass this request to `routing preview --request-stdin --output json`. Submit the same
`change`, the returned `change_digest` as `accept_digest`, `expected_revisions`, and a
new `idempotency_key` to `routing apply --request-stdin --output json`. Wait for its
Operation to reach `succeeded`, then obtain a fresh connection deletion preview.
An uncertain response requires observing or replaying that same Operation/key.
This publishes an unchanged copy of the installed routing configuration at the next
revision, including an empty configuration; it does not create routes or grant access.
Disabled routes, drafts and retained execution versions still block removal.
Desktop exposes this action as **Update references** in the blocked deletion dialog.

## Discover and save a subscription source

```sh
hiroute compute connection options --output json > connection-options.json
```

`data.subscriptions` reports discovery state and candidates exactly. Select one
`connector_owned` candidate, wrap it as `{"candidate": ...}`, and start its check with the
same Preview/Apply rules:

```sh
jq '{candidate:([.data.subscriptions.candidates[] |
    select(.provenance=="connector_owned") | .candidate] | first)}' \
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

For the ordinary path, use Quickstart convenience inputs above and shared authoring defaults. The following advanced JSON example demonstrates an explicitly chosen fixed policy; its limits are not product defaults, and it is not required before difficulty routing:

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

## Configure Agent collaboration

Enable, edit or disable collaboration independently of model settings. Copy the exact Agent ID
from `hiroute agents scan`, then use:

```sh
hiroute agents collaboration configure agent_codex_default --trigger explicit --preview --output json
hiroute agents collaboration configure agent_codex_default --trigger explicit --idempotency-key codex-collab-1 --output json
hiroute agents collaboration configure agent_codex_default --trigger delegate-by-default --idempotency-key codex-collab-2 --output json
hiroute agents collaboration status agent_codex_default --output json
hiroute agents collaboration disable agent_codex_default --preview --output json
hiroute agents collaboration disable agent_codex_default --yes --idempotency-key codex-collab-off-1 --output json
```

`explicit` delegates only when requested; `delegate-by-default` lets the Agent decide.
The workflow also accepts other supported Agent IDs returned by scan. Like Desktop, it selects
the collaboration context: Codex uses `codex_access.root_context_id` even with an occupied
model profile, and other Agents use their own context. Model and access-token settings remain.

After upgrading HiRoute, save collaboration configuration again with the same trigger and a
new configuration idempotency key to update a HiRoute-owned installed Skill. Startup alone
does not replace it. Modified or user-owned Skill files retain the existing conflict rules.

`--preview` runs no local check, writes no submission record and saves no configuration.
A save runs an existing bounded local check only for missing capabilities reported by Preview,
then previews again. It makes no provider model call. Failed checks, file conflicts or remaining
blockers stop the save. Disable uses collaboration's own restore point and requires `--yes`.

Before submission, the CLI freezes the credential-free request in a private file under
`cli-collaboration/` in the persistent CLI state directory listed above, isolated by control
endpoint digest. Runtime cleanup or reboot does not discard it. Keys contain 1–200 ASCII
letters/digits or `. _ : -` and cannot start with `-`. Omit the key to generate one, returned as
`data.cli_submission.idempotency_key`; automation should supply `--idempotency-key` explicitly.
A retry with the same intent and key reuses the exact frozen request. A different intent needs
a new key. After uncertain delivery, follow the returned recovery action or retry with that
same key. `accepted` means the Operation was admitted: use
`hiroute operations get OPERATION_ID --output json` and collaboration status to confirm completion.
A corrupt or incompatible record stops replay; deleting it must not hide uncertain admission.

The advanced JSON interfaces below retain exact context semantics. A collaboration request
using Codex's model profile still returns `CAPABILITY_DENIED`, with
`COLLABORATION_ROOT_CONTEXT_REQUIRED` guidance, instead of silently redirecting the request.

## Hermes profile discovery and connection

Select the absolute effective Hermes profile root explicitly. Agent discovery uses
`agents scan --request-stdin --output json` with
`{"native_target":{"kind":"hermes","config_root":"/ABSOLUTE/HERMES_PROFILE"}}`.
Use the returned context ID in the shared settings Preview/Apply workflow and
retain the same `native_target` in the settings spec.

For source discovery, use `compute scan --request-stdin --output json` with
`{"schema":"hiroute.compute-scan-request/v2","native_target":{"kind":"hermes","config_root":"/ABSOLUTE/HERMES_PROFILE"}}`.
Choose a returned connection's opaque discovery reference and revision, then use
`compute connection prepare` with that discovery and a stable `prepare_id`.
This preparation privately rereads source credentials; no manual credential copy
or model call is required. Select all returned selectable models initially, or an
explicit subset requested by the user. Save an exact nonempty
`selected_model_refs` array through compute Preview/Apply. Catalog candidates are
distinct from native declarations and account entitlement; refresh never expands
an accepted selection. Empty scan input retains the released V1 shape and
one-model handles.

Set `model.intent` to `configure` with
`{"mode":"hermes_default","plan_id":"PUBLISHED_PLAN_ID","protocol":"responses"}`
or choose `chat_completions` / `messages` explicitly when supported. Native Chat
uses the existing OpenAI Chat SDK and the Gateway Chat path; a Chat upstream must
not be relabeled Responses to bypass request-fidelity checks. Normal Apply installs/enables the bundled
adapter together with the default/provider settings. The selected native SDK and
interfaces must support that transport. Saved state does not prove loading in a
running Hermes process; follow the reload/selection guidance.

For native Chat, Hermes v0.21.7 or newer is recommended to preserve reasoning
content across tool calls. Compatible older versions, including 0.19.1, can connect;
missing optional reasoning replay is not a save gate. Only incompatible required
plugin/configuration interfaces or the selected SDK's header delivery block admission.
Source distributions may report `0.0.0`, so actual integration interfaces are checked
independently of version labels. HiRoute does not upgrade Hermes automatically or
change native reasoning payloads. Passive scanning, management preparation and safe
restore retain their own capability checks.

Prepare the management Skill independently with
`"management_skill":{"intent":"configure","settings":{}}` and `model: keep`.
Use the shared restore workflow for either facet; model restore keeps an
independent Skill. Hermes can also independently configure the shared collaboration
facet with `{"trigger_mode":"explicit"}` or `{"trigger_mode":"delegate_by_default"}`,
keeping `model` unchanged. It uses public Worker commands with its existing native
model; no HiRoute model plugin is required. Hermes remains unavailable as a Worker
executor.
The [native guide](../crates/integrations/src/agents/hermes/README.md) explains
ownership, support and native channel/session selection.

## Connect, check, and restore Codex

```sh
hiroute agents scan --output json > agents.json
hiroute agents check agent_codex_default \
  --scope native-authentication --output json
```

`configuration`, `native-authentication`, and `collaboration` are bounded local
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

## JSON request input

Every command that accepts `--request-stdin` also accepts `--request-file PATH`:

```sh
hiroute routing preview --request-file route-preview.json --output json
# Existing pipelines and shell redirection remain supported:
hiroute routing preview --request-stdin --output json < route-preview.json
```

Choose exactly one source. Both use the same bounded UTF-8 JSON parser and typed
command schema; unknown fields, duplicate object members, trailing JSON, repeated
sources and mixtures with human options return `INVALID_ARGUMENTS` before contacting
Local Control. A file must be regular. Relative paths resolve from the calling
working directory; prefix a filename beginning with `-` with `./`. Quote paths
containing spaces. Existing byte limits remain in force (normally 1 MiB; operation
lookup and Worker dependency selection use 64 KiB). A request file never replaces
the separate protected-input or inherited capability channel. `worker --file`
continues to carry prompt text under its existing contract.

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

A positional `sessions receipt RECEIPT_ID` retains the v1 RoutingReceipt query.
To read a v2 Facts page, use the exact logical `request_id` returned by the v2
session timeline (a session ID or v1 receipt ID is a different locator):

```json
{
  "schema": "hiroute.observation.query/v2",
  "intent": {
    "view": "facts",
    "query": {"request_id": "REQUEST_ID_FROM_TIMELINE", "limit": 100, "cursor": null}
  }
}
```

Save this as `facts-request.json`, then run
`hiroute sessions receipt --request-file facts-request.json --output json`.
`--request-stdin` accepts the identical request. The response is a v2 Facts page;
use its returned cursor with the same request ID to read further pages.

`sessions show` returns facts and timeline by default, not conversation bodies. Ordered
RoutingReceipt facts such as `route_decision`, `attempt_started`, and `usage_and_cache` record
the actual plan, model choice, Attempt, and known upstream-reported tokens. `value show`
returns only existing ledger value. Without trustworthy price evidence, an amount stays
`null`; unknown amounts or tokens without value-ledger rows are never fabricated as zero
cost. Same-user Local Control can explicitly read bodies, search, catalog, and ancestry
without a second read ticket. Supplied grants still bind their precise operation and scope;
run-scoped callers retain their existing limits.

`observation plan-quality samples` returns the same stage facts used by the plan's
competence view. Each sample's `branch_execution` identifies its actual model group,
candidate position and frozen judgment policy. Missing or partial scores remain distinct
from zero; a stage score does not rewrite its opening selection reason. Use the returned
cursor to read further pages. This facts-only query does not call a model or grant access
to conversation bodies; discover its filters through the command's `--help` or `schema show`.

Current normal-argument calls also opt into `routing_observations`: all retained
matching C decisions, separate requested/unavailable/omitted/inherited/anomaly
counts, historical protection opportunities and suppression, with exact cohorts
and coverage. Zero denominators or coverage gaps have unavailable rates. Existing
released v2 structured requests retain the prior
response field set. Content-only deletion retains safe evidence; full fact deletion
and retention remove it. These rates do not establish accuracy or model quality.

## Limit active local observation storage

The default detail retention remains seven days. Optionally save a capacity
threshold in bytes, or disable early capacity expiry:

```sh
hiroute observation storage show --output json
hiroute observation storage set --max-active-bytes 20000000000 --output json
hiroute observation storage set --disable --output json
```

These commands use the owner-authenticated local OS channel. A supplied delegated
or read-only grant cannot authorize a policy change.

The example is 20 decimal GB. Exceeding the threshold may expire the oldest
eligible completed-request detail before seven days. Active requests, live shared
content, replay/deletion barriers, non-content monetary aggregates and recovery
backups remain protected. Disabling stops future uncommitted early retirement;
it does not restore expired history.

This is a soft threshold: reclamation uses spaced, bounded batches and backs off
after contention or ineffective work. Initial inventory, pending cleanup or protected
data can delay physical release. `show` reports the saved limit, measured active
allocated bytes, measurement time and pending/blocked reason. Reusable SQLite
pages are not reported as released disk space. Protected source directories and
recovery packages have separate retention; their space must be budgeted for an
upgrade. See [whole-group recovery](release-contracts.md#upgrade-and-manual-recovery).

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

Codex Workers require the selected `codex-acp` adapter to advertise AIR
`sessionFailure` during initialization. The inspected 1.1.5 adapter lacks this
capability; 2.1.1 advertises it. HiRoute checks the capability, not a version
allowlist. If a task fails with `CapabilityUnavailable`, use
`hiroute worker read --run RUN_ID` to inspect its saved progress. A `[HiRoute]`
message identifies this missing capability and explains how to recover: update
the selected adapter, select its paths again in Worker dependencies, then submit
the task again. Use a new submission key for a new run; replaying the previous
key returns the existing failed run. Discovery/selection is not a compatibility
preflight and does not install or update the adapter.

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

The CLI writes no local submission receipt, needs no writable receipt directory, and ignores
`HIROUTE_WORKER_RECEIPT_DIR`. Existing receipt files are left untouched. Prefer a unique literal
`--submission-key` chosen before each new submission and retain it with the original request.
An omitted key is generated and returned by the CLI; a crash before receiving that output can
lose it. If the key is lost, `hiroute worker list --title TITLE --output json` can help locate a
task: follow all pages and verify its time, Plan, cwd and details. Titles can repeat and are
visible only while the initial input remains available (seven days, or until earlier content
deletion), including after completion. If no unambiguous match exists, ask for confirmation;
never automatically create a replacement task from an empty search result.

## Gateway listener

```sh
hiroute gateway show --output json
hiroute gateway set --address 127.0.0.1 --port auto --output json
hiroute gateway set --address 192.0.2.10 --port 8317 \
  --accept-remote-risk --output json
hiroute gateway recover --output json
```

`set` saves desired state, restarts, and advances applied state only after real readiness.
The first automatically chosen port is persisted. Repeating `--port auto`, switching
from fixed to automatic, or changing the address retains the saved desired port
(or the last applied port if no desired port has been selected). A port is selected
only when neither exists; a binding collision fails without choosing a replacement.
If the service-manager restart fails, desired state remains saved and the operation
remains failed. A later `hiroute service run` applies that desired state after readiness.
`recover` selects the current applied listener, not an earlier historical address.

`gateway show` checks public Agent settings status against the current runtime endpoint.
Its `agent_update.required` hint requests a check/reconfiguration for drift, pending
changes or unavailable inspection; it does not depend on the last listener operation's
success. A null `existing_agent_count` means discovery was unavailable. Configured
targets that agree and collaboration-only targets do not require model updates.
Unconfigured/restored model targets also check any saved V1 connection; a missing
old record needs no update, while a disconnected configuration requires a check.
Current configured settings take precedence over superseded connection history.
After a real endpoint change, use fresh
`agents connect preview` / `agents connect apply` with the intended model selection;
HiRoute does not rewrite native Agent configuration during a listener restart.

Local management clients see a
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

Desktop provides an independent HiRoute management control for Codex, Claude Code,
Qoder, Pi, DeepSeek Harness and Hermes. Public CLI uses the same settings interface:
set `management_skill` to `{"intent":"configure","settings":{}}` with `model` and
`collaboration` kept, or configure both Skills in the same request. Preview/Apply
retains its normal exact revisions, digests and idempotency key. Use the returned
effective context; Hermes also retains its explicit `native_target`. Current CLI
status reads include Skill scope, shared/external ownership and unconfirmed loading;
raw status requests can opt in with `include_skills: true`. Restore only the selected
facet's returned reference. A retained external/shared file may remain available
natively after this context releases its reference.

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

Desktop manages these connections under **Models → Decision models**. A native decision model
can reuse the Keys of an existing Native API connection or keep an independent Key. OpenAI
Decisions and System One use explicit wire protocols; they share the same routing questions,
continuous scoring and selection rules. Custom extensions remain a separate connection type.

### Reuse an API connection

Discover the installed presets and locally available sources, then create a decision connection:

```sh
hiroute decision sources list --output json
hiroute decision connections options --provider openai --output json
hiroute decision connections apply --id routing-decision --name "Routing decision" \
  --provider openai --source SOURCE_ID --apply-file decision-connection-apply.json
hiroute decision connections apply --request-file decision-connection-apply.json
hiroute decision connections list --output json
hiroute decision connections test --service routing-decision@SAVED_REVISION --output json
```

The first `apply` previews and writes a private exact Apply request; inspect it before submitting
that same file. A shared connection uses the selected source's current eligible Keys without
copying them. `ready` means locally enabled with a configured Key, not provider entitlement.
Key changes affect later calls without republishing. Changing the decision model, complete URL,
protocol or source creates a new immutable revision; select that revision in the route and publish
it. Previously published routes keep their selected revision and source.

Use `--endpoint`, `--model` and `--wire-protocol system_one|openai_decisions` to set the exact
provider request. For an arbitrary `compatible` connection, select the protocol explicitly.
Choose the appropriate Bailian Token Plan or workspace endpoint; a preset does not establish
access. Use `--candidate` with protected input for an independent Key instead of `--source`.
Typesafe can continue to use independent configuration. The source API never returns Key values.

After saving and testing, attach the returned revision through the existing route authoring flow:

```sh
hiroute routing preview --plan PLAN_ID --decision-service routing-decision@SAVED_REVISION \
  --apply-file decision-route-apply.json
hiroute routing apply --request-file decision-route-apply.json
```

Saving a connection does not call a provider or publish a route. Testing sends a fixed synthetic
request and may use quota; it does not evaluate real conversations or prove all Keys are usable.
Refresh after a source or service revision conflict and preserve non-secret edits; do not silently
replay a stale change. Discover full current request schemas with `schema show`.

### Existing V1 decision service commands

The released `decision services list/apply/test` commands retain their strict V1 contract. Use
`decision connections list/apply/test` for tagged V2 connections, including shared credentials.
Existing V1 content and published route snapshots remain readable without rewriting their hashes.
The following example shows the legacy V1 request shape:

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

Historical problem applicability is the built-in default for newly published native
System One routes. Background is fixed at eight turns. Use ordinary Preview/Apply;
the compiler freezes complete actual prompts, criteria, thresholds and content
digest into the publication.
Existing frozen routes continue unchanged until their next publication. Local
rules and custom extensions retain their existing behavior.

Assessments are default or custom. `judgment.competence.standard:"default"`
resolves the current default at the next edit/publication; `"custom"` preserves
instructions and all criteria byte-for-byte. Released input without intent
recognizes only exact legacy built-in instructions and all three criteria as
default. Threshold changes do not customize text. Reset to default restores the
current default and keeps thresholds. Unsupported state references in custom text
fail Preview with their scope and field; existing publications stay active.

Connectivity keeps `hiroute.classifier-decision-test/v1` and its original result.
The connection page has one test, using a current-default synthetic example.
The separate [applicability diagnostic](../contracts/cli/historical-applicability-test.v1.schema.json)
is available through CLI and acceptance tooling, using
`schema:"hiroute.historical-applicability-test/v1"`, `service_id`,
`service_revision`, `protocol:<actual frozen route>` and `shape:"standard"` or
`"max_branches"`. Read the frozen object from publication Preview. A maximum
sixteen-category dual-group shape has nineteen questions in one request; a smaller
success proves no maximum capacity. Inspect result state and exact machine
identity. Explicit tests may consume quota. Opening, saving and publication never
probe. The route editor has no separate test or additional confirmation.

Observation queries retain `hiroute.observation.query/v2`. Facts and plan-quality
requests may set `include_routing_observations:true`; omission/false preserves the
released strict result fields, authorization encoding and cursor binding. Protocol
identities/digests and shape counts remain available in machine output.

Plans use the existing `routing preview/apply` transaction with `hiroute.plan-editor/v2`:

| Mode | Editor fields |
| --- | --- |
| `smart_saving` | `smart.classifier`, `smart.economy`, `smart.primary` and `smart.judgment` |
| `custom_branches` | `branch_routing.classifier`, `branch_routing.branches`, `branch_routing.default_branch_id` and `branch_routing.judgment` |

Difficulty routing and custom branches judge the group again for each new user message,
then prefer a valid held candidate only within that group. This is automatic.
The optional deprecated `reselect_on_user_message` field is accepted as a no-op
for supported older callers; new requests can omit it. Serialized legacy values
may remain in readback to preserve sealed Plan and publication consistency.

`classifier` is `{kind: "decision_service", service: <complete saved connection revision>}`
for a decision model or custom extension. Only Difficulty routing may instead use
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


### Persist a subscription proxy

Use `hiroute subscription-proxy show --output json` to inspect the saved policy and
whether it has been applied. Save a manual HTTP(S) proxy with:

```sh
hiroute subscription-proxy set --mode manual --url http://127.0.0.1:1187 --output json
hiroute service restart --output json
```

An optional `--no-proxy example.com,192.0.2.0/24` adds bypass entries; local
addresses always bypass. Use `--mode direct` to clear inherited proxies, or
`--mode inherit` to retain the existing service launch-environment policy.
Saving does not interrupt the service; an explicit restart applies it and may
interrupt active requests and tasks. The policy covers all managed subscriptions
and Claude subscription checks, not Worker tools or Native API connections.
Manual URLs cannot contain credentials. PAC and ALL_PROXY are unsupported.
An applied policy is not proof of remote availability: check the subscription and
complete a real inference request separately.

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

The installed CLI uses authenticated same-OS-user Local Control. Explicit content/search needs no
Desktop-created read grant; supplied grants are still validated. Gateway model keys grant no management access.
Use the installed management Skill and command help to choose a path:

| Goal | Initial command |
| --- | --- |
| Recent estimated savings | `hiroute value show --view summary --period 7d` |
| Most expensive sessions | `hiroute value show --group-by session --order-by cost-desc --period 7d` |
| Actual model transitions and reasons | `hiroute sessions list --view switches --period 7d` |
| Find conversations by content | `hiroute sessions list --view conversations --period 7d --query TEXT` |

Keep the returned `analysis_scope` for related views and narrower drill-downs:

```sh
hiroute sessions show SESSION_ID --view routing --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --analysis-scope TOKEN
hiroute value show --group-by request --order-by cost-desc --session SESSION_ID --analysis-scope TOKEN
hiroute sessions show SESSION_ID --view conversation --request REQUEST_ID --analysis-scope TOKEN
```

Choose one time mode: `--period today|yesterday|7d|30d`, `--day YYYY-MM-DD`, or `--from-ms N --to-ms N`.
Civil dates follow the service host's local day; absolute bounds are UTC milliseconds and half-open.
Report both returned requested/effective bounds and boundary offsets. Retention can clip the start to keep
covered evidence available for the scope's lifetime; a range without retained evidence is not complete coverage.
Follow `next_cursor` with the same view, filters and limit, without a new time selector. Page lengths and
bounded member previews are not totals. Follow-ups inherit the relation/numeric/price watermarks and can only
narrow the initial scope. New requests, settlement, unrelated compaction and price edits do not silently
change its totals. Explicit affected deletion, proof withdrawal, authority change or expiry requires a fresh
initial query; keep the old and refreshed results distinct.

Selection and competence stages explain decisions, but stage boundaries are not switches. `between_turn`
requires proven adjacent completed user turns. `continuation_boundary` describes verified native compression,
including compression within a turn; it carries `turn_context_reset` and is not an adjacent-turn change.
Missing proof leaves physical sessions separate. Credential relay and within-request fallback are separate
kinds. Compare actual accepted endpoints and explicit reasoning settings, not whole-profile digests. Open
linked physical members, request IDs and receipts; count each request and attempt once.

Conversation pages return bounded messages/tools. For `reference_only` or partial entries, the installed
`sessions.show` schema describes V2 catalog/content pagination using their returned IDs. `sessions receipt`
opens immutable facts and `observation plan-quality samples` opens recorded stages. Legacy `sessions list
--include-unlinked --query/--model-switch` and `sessions show --content messages-and-tools` keep their meanings;
`--model-switch` is invalid for the new views. No analysis read calls a model or changes routing.

Cost ranking needs no reference model. Select one currency and recorded valuation kind, or choose
`--basis api-equivalent` to recalculate recorded attempts with current source-specific prices. Summary defaults
to that estimate and an unambiguous primary binding; resolve returned reference/currency choices explicitly.
Do not mix recorded estimates, current-price recalculation, currencies or valuation kinds. Full-scope totals
are computed before pagination. Missing usage/prices are unknown, not free/zero; incomplete rankings are
`highest_known_only`. Savings reports signed paired subtotals and missing/excluded coverage, with no percentage
for a zero denominator. Decision overhead remains unmeasured. Coding Plan API-equivalent estimates prove
neither cash savings nor equal output quality, and do not rewrite frozen receipts or subscription billing.

A pending index, missing capture or deleted/expired content is a coverage gap. `pending` reconciliation reports
progress, last-progress and stall information. Retryable read errors distinguish `timeout`, `reader_busy`,
`index_unavailable` and `storage_unavailable`; they are not empty results. Inspect task/answer/tool evidence,
decision source/group/probability/threshold, competence and reasoning before proposing policy changes. Keep a
fresh complete before-image and fully populated recovery commands before applying an authorized change.

The legacy explicit-reference report remains available with its original response/time semantics:

```sh
hiroute value show --routing PLAN_ID --period 7d --basis api-equivalent --reference-binding PRIMARY_BINDING --currency CNY --output json
```

It has no day grouping. A complete full-scope routed estimate can be displayed independently when
reference evidence is missing; the reference total and difference still require complete pairs for
the entire scope. Comparison coverage and paired subtotals describe those complete pairs. Ordinary
`value show` retains its historical valuation meaning.

For missing rates, read the installed `prices.effective` and `prices.override.preview` schemas. Query the exact Binding locator with currency and `valuation_kind: "api_equivalent"`; copy the returned `edit_context` revisions into a price Preview. Manual `rates` use integer currency micros per million tokens, with separate input/output/cache rates. Use verified prices or ask for these non-secret values; do not invent them. Apply the exact returned spec/digest/revisions with one retained key. Before changing a price, retain its prior setting and give fully populated price-restore Preview/Apply commands; `follow_catalog` restores catalog lookup when no manual override existed. API-equivalent prices can be set for subscriptions without changing their entitlement or ordinary billing class. Free, subscription, paid and unknown remain distinct.

When cache creation is charged at the ordinary input rate, set its reference rate equal to input. The amount then does not depend on an unreported input/creation split; HiRoute can combine equal-rate calculation buckets without filling or rewriting recorded usage. Different rates still require the missing count.

New historical valuations can also combine an unreported split when the attempt's own frozen
input and cache-write prices are equal and inclusion and exclusivity are established. The amount
may be complete while the individual cache-write tokens remain unknown. For retained older records,
the session timeline can derive that amount from matching frozen evidence without modifying the
original settlement; older settled totals and expired records are not automatically recalculated.

If the full-scope routed amount is known, input and cache reads are reported, and the input/cache
inclusion and exclusivity are established, a reference with an unreported cache-write split may
show bounds. The same-usage scenario assumes reference input and reads match the observed values
and places unreported writes between zero and input minus reads. These are estimates at the selected
reference prices, not the reference model's observed cache, output or bill; comparison coverage
remains incomplete. Missing other necessary usage, prices or scope evidence prevents the bounds.

Custom-extension routes return the category/group decision directly and publish no HiRoute judgment policy. Parked draft settings return when a built-in connection is selected. Optional extension scores are observation-only.
