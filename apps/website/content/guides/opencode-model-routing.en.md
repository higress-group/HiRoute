# OpenCode model routing: reuse API settings and select a route in /models

Already have an OpenCode provider? Reuse its API in HiRoute, use a fast model for reading code and a stronger one for analysis, then select the route in OpenCode's `/models` picker alongside your existing models. Use Pi when you want a separate check.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/opencode-model-routing/ and help me get models, configure smart routing and connect my Agent. Use this guide's lightweight Pi example to show how a scoped task can use fewer primary-model tokens. Enable delegation when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Discover and save APIs: hiroute compute scan · compute connection prepare / preview / apply
- Create a route and add it to OpenCode: routing preview / apply · agents connect preview / apply
- Check model choices and another executor: sessions timeline · worker plans · worker result
The UI and CLI share configuration and records. Inspect the current setup first and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

Open **Models → General models → Add models → Scan this device**. Select detected OpenCode static providers, model declarations and API key sources, then check and save them.

OpenCode's `/models` may also include dynamically discovered or OAuth plugin models. If the scan misses a source that provides an API address and key, add them under **New API connection**, check the connection and save. You can also import Codex or Claude Code models through **Connect subscription**, or use models you already connected.

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “OpenCode code review” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

Open **Agent → OpenCode → Model routing**, check “OpenCode code review”, choose a compatible protocol and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-opencode-en.webp"><img src="/guide-assets/agent-guide-opencode-en.webp" alt="Adding a published HiRoute route to OpenCode" width="1200" height="900" loading="lazy" /></a>
  <figcaption>The route appears in OpenCode's model list for selection through /models.</figcaption>
</figure>

Restart OpenCode and select the new route with `/models`. Ask it to summarize a small change, then inspect model choices in HiRoute's **Sessions → Routing timeline**.

<h2 id="task-delegation">4. Explore task delegation</h2>

Before renaming an API field, let lightweight Pi find the callers, docs and tests that refer to it. An economical model handles the repository search. OpenCode receives the affected locations and keeps its primary model focused on implementation and verification.

Create a Fixed model route called “Pi rename impact”. Under **More settings → Task delegation**, allow delegation and choose Pi, describe its purpose and enable it. Then turn on **Agent → OpenCode → Task routing**.

> Use HiRoute to ask “Pi rename impact” where this field is referenced. Group the paths and line numbers by callers, docs and tests.

Start with one field and see whether the report gives your main Agent enough to start editing. [Set up lightweight Pi delegation](/en/docs/task-routing/#lightweight-pi)
