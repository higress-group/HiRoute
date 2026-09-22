# Run HiRoute headless on Linux

HiRoute headless installs the local service, Gateway, CLI, and Agent management Skill for the current user. It is designed for servers, remote workstations, and automation environments without a desktop UI. It is not a reduced read-only client: model sources, smart routes, Agent connections, session observation, and task delegation are all available through the CLI.

The current public product validation covers Linux on `x86_64` and `aarch64`. Installation needs no `sudo`; it requires `curl`, Python 3, and a glibc Linux environment capable of running the HiRoute binaries.

## 1. Install

After the first stable Linux package is published, run:

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

The installer downloads the latest stable Linux release and checks its architecture and file integrity before installation. Desktop and standalone must not manage the service in the same `HOME`.

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

## 4. Complete headless setup

Build the first route in this order:

1. Inspect available sources with `compute scan/list/show`. For a custom API, use `compute connection options/test/preview/apply`; use `authorize` for a source that requires browser authorization.
2. Check candidate model capabilities with `models show`.
3. Read current choices and revisions with `routing options`, then create and publish a route through `routing preview/apply`. Confirm it with `routing list/show`.
4. Discover local Agents with `agents scan/list/check`, then connect one through `agents connect preview/apply/status`. Recovery uses `agents restore preview/apply` and removes only fields still owned by HiRoute.
5. After sending a real Agent request, inspect routing facts and usage with `sessions list/show/receipt/status`, and inspect value records backed by available pricing evidence with `value show`.

This is not a second headless control plane. CLI and Desktop reuse the same Application, Local Control, storage, publication, and recovery paths.

## 5. Delegate a task

Once an execution plan has been published, discover it and submit work from a terminal or main Agent:

```sh
hiroute worker executors
hiroute worker plans
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --submission-key first-task-001 \
  -- "Analyze this task, implement it, and run the relevant checks"
```

If a network or process interruption leaves acceptance uncertain, do not retry with a new key. Keep the original `submission-key` and query `worker status --submission ... --operation start`.

## 6. Uninstall

Stop the service, then use the uninstall entry in the same official installer:

```sh
hiroute service stop --output json
curl -fsSL https://hiroute.ai/install/standalone.py | python3 - uninstall
```

Uninstall is limited to version directories, stable entry points, the service definition, and management Skills named by the installation marker. Runtime data is preserved. It refuses replaced stable links, service definitions, or Skills before deletion, but it does not audit every custom change inside an owned version directory; do not store your own files there.

Continue with [HiRoute CLI](/en/docs/cli/) for output, idempotent recovery, and command boundaries, or read [Smart model routing](/en/docs/model-routing/) and [Smart task routing](/en/docs/task-routing/) for the product mechanisms.
