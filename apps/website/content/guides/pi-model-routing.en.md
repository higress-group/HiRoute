# Pi coding agent model routing: reuse your models and route by difficulty

Select a HiRoute route in Pi's `/model` picker and keep your usual tools. Give file reading and symbol searches to an economical model, switch to a primary model for work across modules, and ask Codex CLI for an independent check.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/pi-model-routing/ and reuse my Pi sources in a difficulty route I can select in Pi. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Scan and check reusable settings: hiroute compute scan · compute connection prepare / preview / apply
- Configure a plan and connect Pi: routing preview / apply · agents connect preview / apply
- Prepare and check independent execution: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Start with your Pi model settings

Open **Models → General models → Add models → Scan this device**. HiRoute can read supported `models.json` declarations, API key sources and catalogs from an eligible Pi installation. Check the discovered connection and save the models you want. For a model supplied dynamically by an extension, add its API if available.

You can also start from an existing subscription, API or local model. For example, [connect Qoder models through Magpie](/en/docs/qoder-model-routing/) and use them in Pi.

## 2. Add a difficulty route to Pi's model picker

Create “Pi daily coding” and choose **Difficulty routing**. Use Regular models for symbol searches and code collection, and Primary models for design changes and hard-to-reproduce bugs. Start with **Heuristic rules**, or add a [decision model](/en/docs/model-routing/#difficulty-routing) if you want stage scoring.

Publish the route and add it under **Agent → Pi → Model routing**. Choose the protocol based on the models and the suggestion shown in HiRoute. Restart Pi and select the new route with `/model`; the existing models are still available. Try a small task, then check the HiRoute session timeline.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-pi-en.webp"><img src="/guide-assets/agent-guide-pi-en.webp" alt="Available routes and protocol in Pi model routing settings" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose which routes Pi should offer. Restart Pi and select one with /model.</figcaption>
</figure>

## 3. Let Codex CLI verify the work independently

Use the same models in a new “Codex regression check” plan. Open **More settings → Task delegation**, select **Codex CLI**, and give it a purpose such as “Reproduce a failure and run the relevant tests.” Check the dependencies, publish, and [enable Task routing](/en/docs/task-routing/#delegation-skill) in Pi.

Ask Pi: “Use HiRoute to delegate regression verification for this module to Codex regression check. Run the relevant tests and summarize the outcome.” Check the executor and results in **Task history**. To use Pi as the executor from another Agent, select Pi in a delegation plan and check the Pi CLI and Node.

Next: [Connect Codex CLI](/en/docs/codex-model-routing/) · [Track or continue a delegated task](/en/docs/task-routing/)
