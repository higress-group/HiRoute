# Pi coding agent model routing: reuse your models and route by difficulty

Select a HiRoute route in Pi's `/model` picker and keep your usual tools. Give file reading and symbol searches to an economical model, switch to a primary model for work across modules, and ask Codex CLI for an independent check.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/pi-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Scan and check reusable settings: hiroute compute scan · compute connection prepare / preview / apply
- Configure a plan and connect Pi: routing preview / apply · agents connect preview / apply
- Prepare and check independent execution: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

Open **Models → General models → Add models → Scan this device** and find your Pi sources. HiRoute reads supported `models.json` declarations, API key sources and recognized installed model catalogs. Select a connection, check it and save the models you want.

For models supplied by a runtime plugin, enter the corresponding API address and key under **New API connection**, check it and save. You can also use imported subscription models or [connect Qoder models through Magpie](/en/docs/qoder-model-routing/#connect-qoder).

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Pi daily coding” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

Open **Agent → Pi → Model routing**, check “Pi daily coding”, choose a protocol supported by your models and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-pi-en.webp"><img src="/guide-assets/agent-guide-pi-en.webp" alt="Pi route selection and connection protocol in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Check the published route, save it and select it in Pi's /model picker.</figcaption>
</figure>

Restart Pi, type `/model` and select the new HiRoute route. Ask it to find a function's callers, then inspect the model choices in **Sessions → Routing timeline** in HiRoute.

<h2 id="task-delegation">4. Explore task delegation</h2>

After planning in Pi, delegate “Run the relevant tests and summarize the failures” to Codex CLI while you continue discussing the implementation. Other main Agents can also send research tasks to Pi.

To get started, bind an execution Agent under **More settings → Task delegation** in a plan and enable **Task routing** in the main Agent that sends the task. The executor uses the plan's models, and results appear in **Agent → Task history**. [Continue with the Agent routing guide](/en/docs/task-routing/)
