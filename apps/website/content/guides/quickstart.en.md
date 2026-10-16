# Quickstart

Choose macOS Desktop or Linux headless, connect model access and create the route you need: fixed model or smart saving directly. Smart saving can use explicitly selected local rules without an external decision service. Task delegation is independent.

## 1. Choose how to run HiRoute

- **macOS Desktop:** requires macOS 15.0 or later. Public installers are self-signed builds. Verify the SHA256 after downloading, then follow [Install and open HiRoute on macOS](/en/docs/install-macos/).
- **Linux headless:** designed for servers, remote workstations, and automation. It installs for the current user and needs no `sudo`. Follow [Run HiRoute headless on Linux](/en/docs/install-linux/) to start the service and continue through the CLI.

Steps 2–5 below show the visual Desktop path. Linux users complete the corresponding flow through the same production capabilities in the headless guide. In either case, confirm that the local service is ready first; model routing, session records, and delegated execution all depend on it.

## Use the CLI or an Agent

Run `hiroute quickstart` to inspect the current service, exact targets and reusable sources/Plans. Continue with `--context CONTEXT_ID --plan PLAN_ID`; for a specific Hermes profile, add `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`. The entry is read-only and shows missing choices and next commands.

The main path is source → route → target Agent → native activation → matching request evidence. Existing Preview commands accept concise choices and `--apply-file PATH`, so callers do not hand-write a full editor or transfer digests. See [CLI Quickstart](/en/docs/cli/) for direct smart saving, adjustment, uncertain-response recovery and independent facet restoration. The same management Skill can run in any supported Agent and manage another target while leaving its own model native.

For model decisions, start with `hiroute decision services options` and reuse a suitable saved connection. Enter credentials in your own terminal through the hidden prompt, then let the management Agent continue saving, testing and binding the route. Never send keys in chat. See [CLI decision-service onboarding](/docs/cli/).

## 2. Connect a model

Open Models → General models and select Add model. You can connect a supported local subscription, or open Advanced connection and choose Custom API. General models execute your tasks; this connection is enough for your first route.

Check the connection, select the model you want HiRoute to use, and save it. “Ready to route” means that the connection has the facts required by routing; it does not claim that every upstream model has completed a live inference test.

## 3. Create your first smart route

Open Smart routing and select New smart routing:

1. Enter a name and purpose, such as “Daily development” and “Routine code changes.”
2. Choose Fixed model with one candidate, or Smart saving with economy/primary groups and an explicit decision method.
3. Select Enable. Only published and enabled routes handle new requests.

For further configuration, use Smart saving, Custom branches, or Free first in [Use smart model routing](/en/docs/model-routing/). When you need model judgment and stage assessment, add a built-in connection under Models → Decision models, or connect a self-hosted custom extension.

## 4. Connect your agent

Open Agents and choose the intended installation/profile. For Codex, for example:

1. Open Model routing and select Enable.
2. Turn on Use HiRoute model routing.
3. Select your new smart route and make it the default choice.
4. Save the configuration and complete any checks shown by the page.

HiRoute keeps a restore point. If you disable model routing later, use Connection details and recovery to restore the settings that existed before HiRoute managed the connection.

## 5. Send a real request

Follow the native loading and model-selection guidance, then submit an authorized small task with a clear result. Saved configuration alone is not proof that a running conversation switched. After it starts, open Sessions in HiRoute to inspect the route, actual model, usage, and execution state.

## Next steps

- [Use smart model routing](/en/docs/model-routing/) to configure the four routing types, decision models, and custom branches, then inspect turn selection, actual execution, and later stage assessment.
- [Use smart task routing](/en/docs/task-routing/) to delegate independently executable work.
- [Run HiRoute headless on Linux](/en/docs/install-linux/) to install from an empty environment and configure models, routes, and Agents from the CLI.
- [HiRoute CLI](/en/docs/cli/) to manage HiRoute, inspect runtime facts, start tasks, and recover uncertain submissions from Terminal.
