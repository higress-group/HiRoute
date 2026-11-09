# Qoder CLI model routing and subscription setup

Add a HiRoute plan to Qoder CLI's model picker, or bring your Qoder subscription models into HiRoute through headless Magpie. You can use DeepSeek-Flash for routine coding and reserve Kimi-K3.1-Pro for harder turns, then hand off an independent task to Pi, DeepSeek Harness or Codex CLI.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

- [Use an existing route in Qoder CLI](#use-in-qoder) if you already have a published HiRoute plan.
- [Connect your Qoder subscription](#connect-qoder) to find your account's models, create a difficulty route and bind an executor.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/qoder-model-routing/ and help me add an existing route to Qoder CLI or build one with Qoder models from Magpie. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Inspect the Magpie connection: hiroute compute list · compute scan · compute connection test / preview / apply
- Set the Regular group, Primary group and executor: routing options · routing preview / apply · worker executors
- Add a route to Qoder CLI: agents connect preview / apply
- Check the actual run: worker plans · worker result · sessions timeline
The UI and CLI share configuration and records. Let me sign in locally and use the IDs in my catalog. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

<h2 id="use-in-qoder">Use an existing route in Qoder CLI</h2>

Sign in to Qoder CLI. In HiRoute, open **Agent → Qoder → Model routing**, select your published plan and save. Restart Qoder CLI and select the plan with `/model`. It appears alongside your existing models. To make a plan first, [use models you already have](/en/docs/model-routing/) or follow the subscription steps below.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-qoder-en.webp"><img src="/guide-assets/agent-guide-qoder-en.webp" alt="Qoder CLI model routing settings with the Qoder daily coding plan selected" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add the published route to Qoder CLI's model picker.</figcaption>
</figure>

<h2 id="connect-qoder">1. Connect your Qoder subscription models</h2>

[Start the headless Magpie service](/en/docs/magpie-model-sources/#start-magpie), or use your running instance. To connect an account, run `magpie login qoder` and authorize it in the browser. If you use the [community Qoder plugin](https://github.com/magpie-community/plugins/blob/main/registry.json), run `magpie plugin add @magpie-community/opencode-qoder-auth`, then `magpie plugin login qoder` (use `qoder-cn` for Qoder CN).

After signing in, [read the model catalog](/en/docs/magpie-model-sources/#model-catalog) and [save the Magpie API in HiRoute](/en/docs/magpie-model-sources/#save-api) with the complete IDs from your account. If an eligible Agent configuration already points to Magpie, try **Scan this device** under **Models → General models → Add models** first. Start with Responses for the Pi example below.

<figure class="guide-shot-models">
  <a href="/guide-assets/agent-guide-qoder-models-en.webp"><img src="/guide-assets/agent-guide-qoder-models-en.webp" alt="Qoder DeepSeek-Flash and Kimi-K3.1-Pro models connected through Magpie in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>After saving the connection, find DeepSeek-Flash and Kimi-K3.1-Pro in the model list.</figcaption>
</figure>

## 2. Keep routine work fast and reserve your primary model for hard parts

Under **Smart routing → New smart routing**, choose **Difficulty routing** and name the route “Qoder daily coding”:

- **Regular models**: for example, `qoder/dfmodel` (DeepSeek-Flash) for reading code, gathering facts and making small edits.
- **Primary models**: for example, `qoder/kmodel_latest` (Kimi-K3.1-Pro) for debugging, interface changes and design review.
- **Decision method**: start with **Heuristic rules**. Add a [decision model](/en/docs/model-routing/#difficulty-routing) if you want stage scores and primary takeover after a relevant low score.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="DeepSeek-Flash in Regular and Kimi-K3.1-Pro in Primary for the Qoder route" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Use the faster model for routine work and reserve the primary model for harder turns.</figcaption>
</figure>

Use the models available in your catalog and click **Enable** to publish. Then you can [add the route to Qoder CLI](#use-in-qoder) or bind it to an executor for delegated tasks.

## 3. Let Pi work with the same models

In the plan, open **More settings → Task delegation**. Allow delegation, describe the work (for example, “Read a module and propose a refactor”), select **Pi**, check its dependencies and publish. You can reuse the same model groups in separate plans for Codex CLI or DSH, with a different purpose for each. Each delegated plan binds one executor.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-pi-worker-en.webp"><img src="/guide-assets/agent-guide-pi-worker-en.webp" alt="Pi selected as the execution Agent in a HiRoute task delegation plan" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Pi executes the task. The HiRoute plan chooses the model group.</figcaption>
</figure>

[Enable Task routing](/en/docs/task-routing/#delegation-skill) in your usual Agent and try: “Use HiRoute to delegate a tour of this project's modules to Qoder daily coding. List the entry points and tests without editing files.” Open **Agent → Task history** for the executor and result, and **Sessions → Routing timeline** for the models used.

Next: [Try the same pattern with WorkBuddy](/en/docs/workbuddy-model-routing/) · [Set up Pi as an executor](/en/docs/pi-model-routing/)
