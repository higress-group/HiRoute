# Qoder CLI model routing and subscription setup

Discover your Qoder subscription models through headless Magpie, then configure difficulty routing in HiRoute. Use DeepSeek-Flash for routine coding and Kimi-K3.1-Pro for harder turns. Connect the route to Qoder CLI, or follow the Codex Desktop screenshots below if your Qoder setup does not support BYOK.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/qoder-model-routing/ and help me get models, configure smart routing and connect my Agent. Use this guide's lightweight Pi example to show how a scoped task can use fewer primary-model tokens. Enable delegation when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Inspect the Magpie connection: hiroute compute list · compute scan · compute connection test / preview / apply
- Set the Regular and Primary groups: routing options · routing preview / apply
- Connect Qoder CLI if custom models are available. Otherwise use the separate Codex Desktop setup in this guide. Configure the connection with agents connect preview / apply. Open the separate window from HiRoute or use agents desktop launch in the local graphical session.
- Inspect model choices, then choose an executor if delegation is requested: sessions timeline · worker executors · worker plans · worker result
The UI and CLI share configuration and records. Let me sign in locally and use the IDs in my catalog. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#connect-qoder) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#use-in-qoder) · [4. Explore task delegation](#task-delegation)

<h2 id="connect-qoder">1. Scan or sign in to get models</h2>

Use headless Magpie to discover the models available to your Qoder subscription, then connect them to HiRoute. Magpie handles sign-in and exposes the local model API.

[Install the Magpie CLI](https://usemagpie.ai/docs/start) on the host running HiRoute. Choose the startup method for your system.

- On macOS or Linux with systemd, run `magpie daemon install --no-web`, then check the service with `magpie daemon status`. Run `magpie daemon start` to start an installed service.
- On Windows or hosts without service installation, run `magpie serve` in a separate terminal and keep it running.

If the gateway is already running, go straight to sign-in.

Run `magpie login qoder`. If you use the community plugin, run `magpie plugin add @magpie-community/opencode-qoder-auth` followed by `magpie plugin login qoder`. For Qoder CN, use `magpie plugin login qoder-cn`.

Complete authorization in the browser, then query the catalog. Use `curl.exe` in Windows PowerShell.

```sh
curl -fsS -H 'Authorization: Bearer magpie' http://127.0.0.1:3425/v1/models
```

Choose complete IDs returned for your account. For example, `qoder/dfmodel` is DeepSeek-Flash and `qoder/kmodel_latest` is Kimi-K3.1-Pro.

In HiRoute, open **Models → General models → Add models → New API connection → Custom API** and fill in the connection.

| Setting | Value |
| --- | --- |
| Connection name | For example, “Magpie Qoder” |
| API base URL | `http://127.0.0.1:3425/v1`, or your gateway's address |
| Protocol | Responses for the Codex Desktop setup below |
| API key | `magpie` for the default local gateway, or your configured key |
| Models | Select from the returned catalog, keeping the full source prefix |

Click **Check and continue**, select models and save. If the catalog does not appear, set the model directory path to `/v1/models` under **Advanced settings**, or enter the complete IDs you just retrieved. If a supported Agent configuration already points to this gateway, try **Scan this device** to find and reuse the connection.

<figure class="guide-shot-models">
  <a href="/guide-assets/agent-guide-qoder-models-en.webp"><img src="/guide-assets/agent-guide-qoder-models-en.webp" alt="Qoder models discovered through Magpie and saved in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>After saving, Qoder models appear under General models, ready for difficulty routing.</figcaption>
</figure>

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Qoder daily coding” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="use-in-qoder">3. Connect your Agent</h2>

If custom models are available, [connect Qoder CLI](#qoder-cli). You can also [open this route in Codex Desktop](#codex-desktop).

<h3 id="qoder-cli">Use Qoder CLI</h3>

For a Qoder CLI setup that supports custom models / BYOK, sign in to Qoder first. In HiRoute, open **Agent → Qoder → Model routing**, check “Qoder daily coding” and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-qoder-en.webp"><img src="/guide-assets/agent-guide-qoder-en.webp" alt="Selecting a published HiRoute route for Qoder CLI" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add the smart route you just created to Qoder CLI's model list.</figcaption>
</figure>

Restart Qoder CLI, select the route with `/model` and start a task. Inspect the routing choices in HiRoute's **Sessions → Routing timeline**.

<h3 id="codex-desktop">Use Codex Desktop when Qoder has no BYOK entry</h3>

If your Qoder version or plan does not expose custom models / BYOK, use **Codex Desktop with the Qoder models and smart route you already connected**. Choose the “Qoder daily coding” plan in the steps below.

1. Install the ChatGPT / Codex desktop app on macOS or Windows, then open **HiRoute Desktop → Agent → Codex Desktop**.
2. Click **Choose routes and enable**. For an existing connection, click **Edit** beside **Model routing**. Check your published plan. A single route becomes the default automatically; if you choose several, pick your preferred route under **Default selection**. Click **Enable** or **Save settings**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-desktop-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-en.webp" alt="Codex Desktop model routing dialog with the Daily coding route selected" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Select your new route. When it is the only route, the separate window uses it by default.</figcaption>
</figure>

3. Back in the Codex Desktop details, click **Open separate window** in the **Separate window** section. HiRoute opens a desktop window configured to use your route.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-codex-desktop-launch-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-launch-en.webp" alt="Codex Desktop details in HiRoute with the Open separate window button" width="1200" height="900" loading="lazy" /></a>
  <figcaption>After saving the route, click Open separate window at the upper right and start work in the new window.</figcaption>
</figure>

4. Choose a project in the new window and click **New chat**. Confirm or choose your new route in the model picker at the bottom right of the composer. Try “Find this project’s startup entry points and explain how they connect.” In HiRoute, open **Sessions → Routing timeline** to see the models selected.

<figure class="guide-shot-window">
  <a href="/guide-assets/agent-guide-codex-desktop-window-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-window-en.webp" alt="A new chat in the separate Codex Desktop window with Qoder Responses selected at the bottom right" width="2560" height="1640" loading="lazy" /></a>
  <figcaption>Confirm your route in the model picker at the bottom right. This example uses “Qoder Responses”. Click the screenshot to view it at full size.</figcaption>
</figure>

After changing route settings, quit this instance and open it again with the same button.

<h2 id="task-delegation">4. Explore task delegation</h2>

Before adding a feature, let lightweight Pi find the module's entry points, callers and tests. An economical model handles repeated searches. Your Qoder conversation can start design work from a short index with file locations.

Create a Fixed model route called “Pi module index”. Under **More settings → Task delegation**, allow delegation and choose Pi, give it a purpose and enable it. Turn on **Task routing** in the **Qoder** entry for Qoder CLI. If you use the separate Desktop setup above, send delegated tasks from [Codex CLI](/en/docs/codex-model-routing/#codex-cli).

> Use HiRoute to hand this module to “Pi module index”. List its entry points, direct callers and relevant tests with file paths.

Start with one module and compare usage in the main conversation and the delegated task. [Set up lightweight Pi delegation](/en/docs/task-routing/#lightweight-pi)
