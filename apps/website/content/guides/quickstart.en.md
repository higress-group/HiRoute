# Quickstart

Choose how to run HiRoute, connect model access, then create a route and connect your Agent. To try a new model directly, start from **New models** on Home and follow [Try a new model](/en/docs/model-trial/), which chooses models, publishes the route and maps your Agent in one flow. Difficulty routing can use explicitly selected local rules without an external decision service. Task delegation is independent.

## 1. Choose how to run HiRoute

- **macOS Desktop:** requires macOS 15.0 or later. Public installers are self-signed builds. Verify the SHA256 after downloading, then follow [Install and open HiRoute on macOS](/en/docs/install-macos/).
- **Windows Desktop:** use the native x64 installer. It installs for the current user and includes the service and CLI without requiring Python or administrator privileges. Follow [Install HiRoute Desktop on Windows](/en/docs/install-windows/).
- **Linux Web UI and CLI:** designed for servers, remote workstations, and automation. It installs for the current user on x86_64 or ARM64 and needs no `sudo`. Enable the Web UI and open its login URL in a browser to use the same interface as Desktop; the server needs no desktop. Follow [Install HiRoute on Linux](/en/docs/install-linux/).

The [download page](/en/download/) lists available published packages.

Steps 2–5 below apply to Desktop and the Linux Web UI. The browser manages the models, routes, Agents and files on the service host. You can also complete setup through the CLI. Confirm that the local service is ready first; model routing, session records, and delegated execution all depend on it.

## Use the CLI or an Agent

Run `hiroute quickstart` to inspect the current service, exact targets and reusable sources/Plans. Continue with `--context CONTEXT_ID --plan PLAN_ID`; for a specific Hermes profile, add `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`. The entry is read-only and shows missing choices and next commands.

The main path is source → route → target Agent → native activation → matching request evidence. Existing Preview commands accept concise choices and `--apply-file PATH`, so callers do not hand-write a full editor or transfer digests. See [CLI Quickstart](/en/docs/cli/) for direct difficulty routing, adjustment, uncertain-response recovery and independent facet restoration. The same management Skill can run in any supported Agent and manage another target while leaving its own model native.

For model decisions, start with `hiroute decision services options` and reuse a suitable saved connection. Enter credentials in your own terminal through the hidden prompt, then let the management Agent continue saving, testing and binding the route. Never send keys in chat. See [CLI decision-service onboarding](/en/docs/cli/).

## 2. Connect a model

Open **Models → General models** and select **Add models**. You can connect a subscription, scan this device to reuse existing subscriptions and model configurations, or create a new API connection. General models execute your tasks; this connection is enough for your first route. For connecting a subscription and its API together, API key spending allowances and proxy settings, see [Connect models](/en/docs/models/).

Check the connection, select the model you want HiRoute to use, and save it. **Connection ready** means that the connection has the facts required by routing; it does not claim that every upstream model has completed a live inference test.

## 3. Create your first smart route

Open **Smart routing** and select **New smart routing**:

1. Enter a name and purpose, such as “Daily development” and “Routine code changes.”
2. Choose Fixed model with one candidate, or Difficulty routing with regular/primary groups and an explicit decision method.
3. Select Enable. Only published and enabled routes handle new requests.

You can also select **Create routing** in a model's details to create a fixed-model route from that model. For further configuration, use Difficulty routing, Custom branches, or Free first in [Use smart model routing](/en/docs/model-routing/). When you need model judgment and stage assessment, add a built-in connection under **Models → Decision models**, or connect a self-hosted custom extension.

## 4. Connect your agent

Codex is the example here. Open **Agent**, choose the discovered Codex, select **Enable** under **Model routing**, then:

1. Choose a **Connection mode**. **On demand · CLI only** is recommended: it affects only the HiRoute profile, and ordinary Codex and Desktop keep their settings. **Use by default · CLI and Desktop** makes Codex CLI and Desktop that share the configuration directory use HiRoute.
2. Under **Models available in Codex**, select your new smart route, and make it the **Default selection**.
3. On demand, select **Enable and copy launch command**, then start Codex with that command in a terminal. By default, select **Enable model routing**, then restart Codex CLI or Desktop after saving.

When HiRoute is used by default, Codex filters `/resume` by model provider, so earlier sessions may disappear while their files remain. **Find earlier sessions** on the page explains how to get back to them.

For a separate Codex Desktop window on macOS or Windows Desktop, choose the **Codex Desktop** entry on the Agent page. Enable its model routing and save, then select **Open separate window**. Its configuration stays separate from your ordinary Codex profile.

Other Agents connect like this:

- **Claude Code:** start `claude` normally and use its native Opus, Sonnet or Haiku preset for mapped routes.
- **Qoder, Pi, DeepSeek Harness and OpenCode:** HiRoute adds routes you can explicitly select in the Agent's model choices, keeping native models and defaults. Restart the Agent after saving and select a route.
- **Hermes:** first select the configuration directory Hermes actually uses, then choose one route as its default main model.

Keep HiRoute running while you use routing; enabling does not set up automatic startup. HiRoute keeps a restore point. If you disable model routing later, use **Connection details and recovery** to restore the settings that existed before HiRoute managed the connection.

## 5. Send a real request

Follow the native loading and model-selection guidance, then submit an authorized small task with a clear result. Saved configuration alone is not proof that a running conversation switched. After it starts, open the routing timeline in **Sessions** to inspect the route, actual model, usage, and execution state.

## Next steps

- [Connect models](/en/docs/models/) to connect subscriptions and APIs, combine same-named models into one route, and limit API key spending.
- [Try a new model](/en/docs/model-trial/) to let a new model take simple work with a primary model as backup, then adjust trust from evidence.
- [Use smart model routing](/en/docs/model-routing/) to configure the four routing types, decision models, and custom branches, then inspect turn selection, actual execution, and later stage assessment.
- [Use smart task routing](/en/docs/task-routing/) to delegate independently executable work.
- [Install HiRoute on Linux](/en/docs/install-linux/) to configure models, routes and Agents through the Web UI or CLI, locally or from another computer.
- [HiRoute CLI](/en/docs/cli/) to manage HiRoute, inspect runtime facts, start tasks, and recover uncertain submissions from Terminal.
