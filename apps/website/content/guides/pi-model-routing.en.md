# Pi coding agent model routing: reuse your models and route by difficulty

Select a HiRoute route in Pi's `/model` picker and keep your usual tools. Give file reading and symbol searches to an economical model, switch to a primary model for work across modules, and ask Codex CLI for an independent check.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/pi-model-routing/ and help me get models, configure smart routing and connect my Agent. Use this guide's lightweight Pi example to show how a scoped task can use fewer primary-model tokens. Enable delegation when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
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

If a runtime plugin provides an API address and key, enter them under **New API connection**, check the connection and save. You can also import Codex or Claude Code models through **Connect subscription**, or [connect Qoder models through Magpie](/en/docs/qoder-model-routing/#connect-qoder).

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

When your main Pi chat uses a primary model for design work, send test discovery and call-site searches to a separate lightweight Pi route. Let the economical model handle file reads and searches, then use the short index in the main chat. For a larger implementation, you can also choose a route bound to Codex CLI.

Create a Fixed model route called “Pi quick search”. Under **More settings → Task delegation**, allow delegation and choose Pi, describe its purpose and enable it. Turn on **Task routing** in the main Pi entry.

> Use HiRoute to delegate test discovery for this function to “Pi quick search”. Return the test files, case names and behaviors covered.

Check the main chat and delegated task separately to see which models they use and how much each consumes. [Set up a lightweight Pi executor](/en/docs/task-routing/#lightweight-pi)
