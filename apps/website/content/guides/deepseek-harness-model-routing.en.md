# DeepSeek Harness model routing and execution Agent setup

Keep the DeepSeek Harness workflow. Use a fast model for straightforward questions and a primary model for deeper analysis. Add a route to DSH Web or Desktop, or have another Agent delegate work to DSH.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/deepseek-harness-model-routing/ and reuse models, set up routing and configure delegation for the DSH entry I use. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find DSH and importable models: hiroute quickstart · compute scan · compute connection prepare
- Publish a plan and connect the right entry: routing preview / apply · agents connect preview / apply
- Check executors and actual models: worker executors · worker result · sessions timeline
The UI and CLI share configuration and records. Distinguish Web from Desktop and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Reuse DSH models or connect an API

For an existing DSH Web/CLI setup, look for an eligible static provider under **Models → General models → Add models → Scan this device**. The provider needs explicit model declarations. Check the discovered connection and save the models you want.

If there is nothing to import, add a DeepSeek API under **New API connection**, or reuse a source already in HiRoute, such as [Qoder, WorkBuddy or another API](/en/docs/agent-integrations/). Pick one model for fast analysis and another for harder reasoning.

## 2. Add a difficulty route to Web or Desktop

Under **Smart routing**, create “DSH analysis” with **Difficulty routing**. Use Regular to read code and extract logs, and Primary to diagnose the cause. Start with heuristic rules; for stage scores, add a [decision model](/en/docs/model-routing/#difficulty-routing). Publish and open **Agent → Model routing**.

Select **DeepSeek Harness Web** or **DeepSeek Harness Desktop** according to the client you use. Their settings are separate. Add the published route, restart that client and select the route in its model picker. Try a small task and confirm the route in **Sessions**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-dsh-en.webp"><img src="/guide-assets/agent-guide-dsh-en.webp" alt="HiRoute model routing dialog for DeepSeek Harness Web" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add the route to the DSH entry you use. Web and Desktop have separate settings.</figcaption>
</figure>

## 3. Send a fix to Codex or use DSH for analysis

To delegate from DSH, use its Web/CLI entry with Task routing. Reuse your models in a “Codex scoped fix” plan. Under **More settings → Task delegation**, bind **Codex CLI**, describe its purpose, check dependencies and publish. [Enable Task routing](/en/docs/task-routing/#delegation-skill) for DSH, then ask it to hand off a well-scoped fix.

To delegate from Codex, Hermes or another main Agent to DSH, select **DeepSeek Harness** as the executor in a separate plan and prepare its CLI dependencies. Open **Agent → Task history** for the executor and outcome, and the session's routing timeline for the models used.

Next: [Delegate to DSH from Codex](/en/docs/codex-model-routing/) · [Set up Hermes with a Codex execution plan](/en/docs/hermes-model-routing/)
