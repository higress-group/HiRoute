# OpenCode model routing: reuse API settings and select a route in /models

Already have an OpenCode provider? Reuse its API in HiRoute, use a fast model for reading code and a stronger one for analysis, then select the route in OpenCode's `/models` picker alongside your existing models. Use Pi when you want a separate check.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/opencode-model-routing/ and reuse my eligible OpenCode API settings in a new difficulty route. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Discover and save APIs: hiroute compute scan · compute connection prepare / preview / apply
- Create a route and add it to OpenCode: routing preview / apply · agents connect preview / apply
- Check model choices and another executor: sessions timeline · worker plans · worker result
The UI and CLI share configuration and records. Inspect the current setup first and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Scan for APIs you already use in OpenCode

Open **Models → General models → Add models → Scan this device**. Check a supported static OpenCode provider, model and API key source and save the models you want. OpenCode's own `/models` list may also include dynamically discovered models or models from OAuth plugins. If the scan misses one, [connect the corresponding API](/en/docs/models/) or use an existing subscription or local model.

## 2. Add an OpenCode difficulty route

Create “OpenCode code review” with **Difficulty routing**. Give Regular the diff summary and file search, and Primary the impact and edge case analysis. Start with **Heuristic rules** or use a [decision model](/en/docs/model-routing/#difficulty-routing) to score stages. Publish.

Under **Agent → OpenCode → Model routing**, select the plan and a compatible protocol. Save, restart OpenCode and select the route in `/models`. Your existing providers and default model stay available. Ask OpenCode to summarize a small diff, then check **Sessions → Routing timeline** in HiRoute.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-opencode-en.webp"><img src="/guide-assets/agent-guide-opencode-en.webp" alt="OpenCode model routing settings with a HiRoute plan selected" width="1200" height="900" loading="lazy" /></a>
  <figcaption>The route becomes another option in OpenCode's model picker.</figcaption>
</figure>

## 3. Give Pi an independent check

Reuse your models in a “Pi independent check” plan. Set the two model groups, open **More settings → Task delegation**, allow delegation and select **Pi**. Set the purpose, check dependencies and publish. [Enable Task routing](/en/docs/task-routing/#delegation-skill) in OpenCode.

Try: “Use HiRoute to delegate a test gap review of this diff to Pi independent check. Report the findings.” Open **Agent → Task history** for the executor and result. If you want another Agent to use OpenCode as its executor, select OpenCode in a separate delegation plan.

Next: [Prepare Pi as an execution Agent](/en/docs/pi-model-routing/) · [Choose another integration](/en/docs/agent-integrations/)
