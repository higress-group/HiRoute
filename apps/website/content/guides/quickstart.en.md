# Quickstart

Choose macOS Desktop or Linux headless, connect one model through a fixed route, then enable smart saving or task delegation when you need it. Getting started does not require an external decision service.

## 1. Choose how to run HiRoute

- **macOS Desktop:** requires macOS 15.0 or later. Public installers are self-signed builds. Verify the SHA256 after downloading, then follow [Install and open HiRoute on macOS](/en/docs/install-macos/).
- **Linux headless:** designed for servers, remote workstations, and automation. It installs for the current user and needs no `sudo`. Follow [Run HiRoute headless on Linux](/en/docs/install-linux/) to start the service and continue through the CLI.

Steps 2–5 below show the visual Desktop path. Linux users complete the corresponding flow through the same production capabilities in the headless guide. In either case, confirm that the local service is ready first; model routing, session records, and delegated execution all depend on it.

## 2. Connect a model

Open Models → General models and select Add model. You can connect a supported local subscription, or open Advanced connection and choose Custom API. General models execute your tasks; this connection is enough for your first route.

Check the connection, select the model you want HiRoute to use, and save it. “Ready to route” means that the connection has the facts required by routing; it does not claim that every upstream model has completed a live inference test.

## 3. Create your first smart route

Open Smart routing and select New smart routing:

1. Enter a name and purpose, such as “Daily development” and “Routine code changes.”
2. For the first route, choose Fixed model and add the model you just connected.
3. Select Enable. Only published and enabled routes handle new requests.

Once this path works, configure Smart saving, Custom branches, or Free first in [Use smart model routing](/en/docs/model-routing/). When you need model judgment and stage assessment, add a built-in connection under Models → Decision models, or connect a self-hosted custom extension.

## 4. Connect your agent

Open Agents, select a detected Codex or Claude Code installation, then:

1. Open Model routing and select Enable.
2. Turn on Use HiRoute model routing.
3. Select your new smart route and make it the default choice.
4. Save the configuration and complete any checks shown by the page.

HiRoute keeps a restore point. If you disable model routing later, use Connection details and recovery to restore the settings that existed before HiRoute managed the connection.

## 5. Send a real request

Return to your usual agent and submit a small task with a clear result. After it starts, open Sessions in HiRoute to inspect the route, actual model, usage, and execution state.

## Next steps

- [Use smart model routing](/en/docs/model-routing/) to configure the four routing types, decision models, and custom branches, then inspect turn selection, actual execution, and later stage assessment.
- [Use smart task routing](/en/docs/task-routing/) to delegate independently executable work.
- [Run HiRoute headless on Linux](/en/docs/install-linux/) to install from an empty environment and configure models, routes, and Agents from the CLI.
- [HiRoute CLI](/en/docs/cli/) to manage HiRoute, inspect runtime facts, start tasks, and recover uncertain submissions from Terminal.
