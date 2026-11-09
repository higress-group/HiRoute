# Install HiRoute on Linux

HiRoute on Linux provides an Omarchy desktop app and a standalone **Web UI and CLI** for servers, remote workstations and automation. Use your browser to connect models, configure routes and Agents, inspect sessions and manage delegated tasks. The standalone service runs without a desktop environment, and the CLI supports the full management workflow.

Check the [download page](/en/download/) for published packages. A complete package includes the local service, Gateway, CLI, Web UI console assets and Agent management Skill.

The standalone packages support `x86_64` and ARM64 (`aarch64`) with glibc 2.31 or later. Standalone installation requires `curl` and Python 3 and runs as the current user.

<h2 id="omarchy">Install on Omarchy</h2>

Choose the signed Arch package for your architecture in the [Omarchy downloads](/en/download/#omarchy). Follow the [Omarchy installation guide](/en/docs/install-omarchy/) to verify signatures and install with `pacman`, then open **HiRoute** from the app launcher. Choose **Install background service** on first run or attach to an existing compatible service. The desktop app and CLI share the same routes, Agents and sessions.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for your Agent">
<div class="agent-guide-heading"><strong>Let your Agent use this guide</strong></div>

Copy these instructions into your Agent conversation and describe what you want to do. They include documentation links, built-in Skill setup and CLI pointers.

<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read the documentation below, then use the HiRoute CLI to carry out my goal for “Install HiRoute on Linux.” Use the goal from our conversation and ask only for missing information you need.

Page documentation: https://hiroute.ai/en/docs/install-linux/
CLI reference: https://hiroute.ai/en/docs/cli/

Follow the guide for the requested installation or checks. On Omarchy, follow the signed package verification and installation steps from the Omarchy download section. Once installed, help me enable the built-in management Skill under “Agent → HiRoute management → Enable” for further configuration: https://hiroute.ai/en/docs/#management-skill

Command pointers (choose only what the goal needs; prefix each with hiroute):
- Start and inspect the service: service start · service status · system status
- Configure and open browser management: console init · console open
- Discover targets and next steps: quickstart · schema list
- Enable management, then configure models and routes: agents connect preview --management enable → agents connect apply · compute · routing

The UI and CLI connect to the same HiRoute service and share configuration and execution records. Inspect and reuse existing state. Get exact parameters from the installed command’s --help and hiroute schema show --command-id COMMAND_ID --include-schema. Follow the built-in Skill for the required preview, apply and verification, then report the result.
```

</details>
</aside>

## 1. Install the standalone service

Run this command to install the latest stable release:

```sh
curl -fsSL https://hiroute.ai/install.sh | sh
```

To inspect the script first:

```sh
curl -fsSLo install.sh https://hiroute.ai/install.sh
less install.sh
sh install.sh
```

The entry points are installed under `~/.local/bin`. If your shell cannot find `hiroute`, add this line to its configuration and open a new terminal:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

The installer downloads the latest stable standalone release and checks its architecture and file integrity before installation.

Use the same installer to upgrade. It checks the storage prerequisites before replacing the program. Keep a matching package and complete data backup if you may need to restore the previous version. After starting the new service, reopen a saved route and make a small request. For startup failures and whole-group recovery, see the [Linux CLI and service reference](https://github.com/higress-group/HiRoute/blob/main/docs/standalone-cli.md#storage-upgrade-recovery).

## Open the Web UI

The Web UI uses HiRoute's authenticated Web console. Linux keeps it off unless configured. You can choose local access and a port during installation:

```sh
curl -fsSL https://hiroute.ai/install.sh | sh -s -- --console-mode loopback --console-port 5838
```

Or initialize it after installation, before starting the service:

```sh
hiroute console init --mode loopback --port 5838
```

For access from another computer, use `--mode network --listen 192.0.2.10 --port 5838 --allow-plaintext-network`, replacing the example IP with the host's reachable address. Wildcard binds such as `0.0.0.0` also require `--advertise-address IP`. The equivalent installer flags use the `--console-` prefix; plaintext consent remains `--allow-plaintext-network`. Upgrades without these options preserve the existing configuration.

After service startup, `hiroute console open` prints a two-minute, single-use login URL for the actual local or remote listener. Open it on your computer; a server needs no desktop. The command never starts listening. Desired changes require a service restart. HTTP remote sessions display an unencrypted-traffic warning. Named CLI-created access tokens remain available for repeated login. See the [console guide](https://github.com/higress-group/HiRoute/blob/main/docs/web-console.md).

## 2. Start the service explicitly

Installation never starts the service or replays a request automatically. After installation, run:

```sh
hiroute service start --output json
hiroute system status --output json
```

`service start` uses the current user's systemd user manager and therefore expects a normal login session. Minimal containers, some SSH environments, or hosts without a user session may not provide one. In that case, run the daemon in a managed foreground terminal:

```sh
hiroute service run
```

Keep that process running and execute `hiroute system status --output json` from another terminal. For operation after logout, enable a systemd user session or linger according to the host's security policy; the installer does not change that host-level setting.

`system status` should report a ready role-all daemon and Gateway. If the service is unavailable, inspect it with:

```sh
hiroute service status --output json
hiroute service doctor --output json
hiroute service logs --output json
```

After setup or an upgrade, verify a saved route with a small request through its connected Agent. See **Send a real request** in the [Quickstart](/en/docs/).

`service`, `gateway`, and `protected-input` are Host management commands. Discover them through root and family help:

```sh
hiroute --help
hiroute service --help
```

## 3. Discover application commands

Models, routes, Agents, sessions, and Worker operations belong to the Application / Local Control layer. Treat the Released schema returned by the installed version as authoritative instead of guessing request fields from a web example:

```sh
hiroute schema list --output json
hiroute schema show --command-id compute.connection.test --output json
hiroute compute connection test --help
```

Start with this copy-pasteable read-only tour; it needs no configuration JSON:

```sh
hiroute compute scan --output json
hiroute compute list --output json
hiroute routing list --output json
hiroute agents scan --output json
hiroute agents list --output json
hiroute sessions status --output json
```

These commands show discoverable sources, existing routes, connectable Agents, and observation status. After identifying real IDs, use the matching `options`, `schema show`, and leaf `--help` to construct a write request.

Writes follow one safety pattern: read schema and options, perform a bounded test, then submit the same change to `preview`. Inspect its digest, revisions, and effects before sending it to `apply`. Pass passwords and API keys through `protected-input`, never through ordinary JSON, arguments, or logs.

## 4. Configure models, routes and Agents

With the Web UI open, follow [Quickstart](/en/docs/#management-skill) to enable the built-in management Skill under “Agent → HiRoute management,” then let your Agent configure models and routes. All paths, Agent installations and native files belong to the service host, including when your browser runs on another computer. Desktop-only controls such as application updates and login-start settings are absent.

For terminal or automation use, build the first route through the CLI in this order:

1. Inspect available sources with `compute scan/list/show`. For a custom API, use `compute connection options/test/preview/apply`; use `authorize` for a source that requires browser authorization.
2. Check candidate model capabilities with `models show`.
3. Read current choices and revisions with `routing options`, then create and publish a route through `routing preview/apply`. Confirm it with `routing list/show`.
4. Discover local Agents with `agents scan/list/check`, then connect one through `agents connect preview/apply/status`. Recovery uses `agents restore preview/apply` and removes only fields still owned by HiRoute.
5. After sending a real Agent request, inspect routing facts and usage with `sessions list/show/receipt/status`, and inspect value records backed by available pricing evidence with `value show`.

Web UI, CLI and Desktop reuse the same service-owned configuration, storage, publication and recovery paths.

## 5. Delegate a task

In the Web UI, open “Agent → Select your main Agent → Task routing → Enable.” Choose when to delegate and complete the built-in delegation Skill check. Then give the main Agent your task goal and working directory; the Skill discovers available routes, executes the task and retrieves the result.

Task history in the UI and `worker list`, `worker status` and `worker result` in the CLI read the same records. See [Agent routing](/en/docs/task-routing/) for illustrated steps and a command map.

## Model information updates

The local service automatically retrieves model
information over HTTPS from `hiroute.ai` and validates its content. The same setting applies to Desktop, the web console,
and standalone use. Requests include no account, installation or usage identifiers.
Retrieval does not call a model or test a connection. Before the first publication,
the service continues with its built-in model catalog.

Inspect the current copy or request a refresh:

```sh
hiroute models feed status --output json
hiroute models feed refresh --output json
```

Turn automatic retrieval off with:

```sh
hiroute models feed settings apply --retrieval off
```

Turning it off keeps the verified copy already held by the service. Remove that
copy through Models → Model feed → Remove copy, where the preview can also turn
retrieval off. A published plan continues to execute its frozen models after removal;
new model selection and default prices use the model facts still available.

## 6. Uninstall

Stop the service, then use the uninstall entry in the same official installer:

```sh
hiroute service stop --output json
curl -fsSL https://hiroute.ai/install/standalone.py | python3 - uninstall
```

Uninstall is limited to version directories, stable entry points, the service definition, and management Skills named by the installation marker. Runtime data is preserved. It refuses replaced stable links, service definitions, or Skills before deletion, but it does not audit every custom change inside an owned version directory; do not store your own files there.

Continue with [HiRoute CLI](/en/docs/cli/) for output, idempotent recovery, and command boundaries, or read [Smart model routing](/en/docs/model-routing/) and [Smart task routing](/en/docs/task-routing/) for the product mechanisms.
