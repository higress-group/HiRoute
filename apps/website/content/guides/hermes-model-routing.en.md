# Hermes Agent model routing: reuse a profile and delegate coding to Codex

Keep chatting and coordinating in Hermes. Reuse model sources from its active profile, route straightforward questions to Regular and deeper reasoning to Primary, and send a scoped code change to Codex CLI.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/hermes-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Confirm the profile and available sources: hiroute quickstart --hermes-root PATH · compute scan · compute connection prepare
- Publish a route and set the Hermes main model: routing preview / apply · agents connect preview / apply
- For task delegation when requested: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Confirm the directory first and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

In **Agent → Hermes**, confirm the profile and configuration directory you use. Open **Models → General models → Add models → Scan this device**. Select the same Hermes directory when needed, then check and save the detected API models.

To add models, enter the service address and key under **New API connection**, check it and save. Existing subscriptions and local model connections can also be used.

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Hermes daily chat” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

Open **Agent → Hermes → Model routing**, confirm the configuration directory and select “Hermes daily chat” as that profile's default main model. Choose the protocol and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-hermes-en.webp"><img src="/guide-assets/agent-guide-hermes-en.webp" alt="Hermes profile directory, default route and protocol" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Confirm the Hermes profile and select your published route as its default main model.</figcaption>
</figure>

Reload Hermes as prompted and open a new conversation. Ask it to organize a short document, then inspect the models used in HiRoute's **Sessions → Routing timeline**.

<h2 id="task-delegation">4. Explore task delegation</h2>

Once you agree on a requirement in Hermes, send “Implement this change and run the relevant tests” to Codex CLI. Hermes can continue the discussion, coordinate the work and summarize the result.

To get started, bind an execution Agent under **More settings → Task delegation** in a plan and enable **Task routing** in the main Agent that sends the task. The executor uses the plan's models, and results appear in **Agent → Task history**. [Continue with the Agent routing guide](/en/docs/task-routing/)
