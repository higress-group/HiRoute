# Hermes Agent model routing: reuse a profile and delegate coding to Codex

Keep chatting and coordinating in Hermes. Reuse model sources from its active profile, route straightforward questions to Regular and deeper reasoning to Primary, and send a scoped code change to Codex CLI.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/hermes-model-routing/ and reuse models from my active Hermes profile, set up difficulty routing and prepare a separate Codex plan. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Confirm the profile and available sources: hiroute quickstart --hermes-root PATH · compute scan · compute connection prepare
- Publish a route and set the Hermes main model: routing preview / apply · agents connect preview / apply
- Follow Codex execution: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Confirm the directory first and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Find the profile Hermes actually uses

Check the configuration directory under **Agent → Hermes**. If you use multiple profiles, pick the one that handles your everyday conversations. Go to **Models → General models → Add models → Scan this device**, select the same Hermes profile when needed, and check and save any eligible API models it finds.

If the model you want is absent, add an API or reuse an existing source. For example, combine [Qoder subscription models](/en/docs/qoder-model-routing/) with your usual primary model.

## 2. Make a difficulty route the Hermes default

Create “Hermes daily chat” with **Difficulty routing**. Put short questions and fact extraction in Regular, and multi-step planning in Primary. Start with **Heuristic rules** or add a [decision model](/en/docs/model-routing/#difficulty-routing), then publish.

Return to **Agent → Hermes → Model routing** and set the plan as the default main model for the selected profile. Reload Hermes as directed. If an existing conversation still has another model selected, try a new conversation and check the route and actual model in **Sessions**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-hermes-en.webp"><img src="/guide-assets/agent-guide-hermes-en.webp" alt="Hermes model routing settings showing the profile and default main model plan" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Check the configuration directory first, then select the default route for that Hermes profile.</figcaption>
</figure>

## 3. Hand a coding task to Codex from Hermes

Create a “Codex coding” plan. Reuse the same models or pick Regular and Primary models for coding. Under **More settings → Task delegation**, select **Codex CLI**, describe its purpose (“Implement scoped changes and run relevant tests”), check dependencies and publish.

<figure class="guide-shot-panel">
  <a href="/guide-assets/routing-worker-en.webp"><img src="/guide-assets/routing-worker-en.webp" alt="Codex CLI selected as the execution Agent in HiRoute task delegation settings" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose Codex CLI in the execution plan so Hermes can hand it suitable work.</figcaption>
</figure>

[Enable Task routing](/en/docs/task-routing/#delegation-skill) for the Hermes profile. Try: “Use HiRoute to delegate this small change to Codex coding. Summarize the changes and tests afterward.” Open **Agent → Task history** to see Codex's result. Hermes can keep its original main model if all you need is delegation.

Next: [Set up Codex CLI](/en/docs/codex-model-routing/) · [See the Hermes case study with several executors](/en/news/hermes-super-agent/)
