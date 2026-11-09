# Claude Code model routing with Opus, Sonnet and Haiku presets

Keep coding in Claude Code and open a HiRoute route through a familiar model preset. Let an economical model handle easy turns and a primary model make the hard decisions. When you want an independent check, use Pi to run it.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/claude-code-model-routing/ and help me reuse Claude models and map a difficulty route to the preset I select. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find a subscription or API: hiroute compute scan · compute connection options / prepare
- Create the plan and preset mapping: routing preview / apply · agents connect preview / apply
- Configure an executor and check results: worker executors · worker plans · worker result · sessions timeline
The UI and CLI share configuration and records. Let me authorize the connection. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Connect a Claude subscription or Anthropic API

Open **Models → General models → Add models**. **Scan this device** can find supported Claude Code subscription and static API settings. Check a detected subscription when prompted, or sign in independently under **Connect a subscription**. If you have an Anthropic API key, use **New API connection**.

Pick two models that suit your work. For example, use Haiku for routine code reading and Sonnet or Opus for a fix that needs deeper reasoning. Choose IDs available to your account. See [Anthropic's gateway documentation](https://code.claude.com/docs/en/llm-gateway) for its supported API scope.

## 2. Map the Sonnet preset to a difficulty route

Under **Smart routing**, create “Claude daily coding” and choose **Difficulty routing**. Add your Regular and Primary models. Use **Heuristic rules**, or add a [decision model](/en/docs/model-routing/#difficulty-routing) for stage scoring. Publish the plan.

In **Agent → Claude Code → Model routing**, expand **Advanced settings** and map Sonnet to this plan. The Opus, Sonnet and Haiku presets can each keep their native mapping or point at a plan you choose. Restart `claude`, select the mapped preset with `/model` and try a small task. See the actual model in HiRoute's **Sessions** view.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-en.webp"><img src="/guide-assets/agent-guide-claude-en.webp" alt="HiRoute model mappings for the Opus, Sonnet and Haiku presets in Claude Code" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Start with one preset. You can leave the other presets on their usual models.</figcaption>
</figure>

## 3. Ask Pi to investigate independently

Reuse your connected models in a “Pi code investigation” plan. Give file search and evidence gathering to Regular and cross-module analysis to Primary. Under **More settings → Task delegation**, select **Pi**, describe the purpose, check its CLI and Node setup and publish. Use a compatible protocol and models that support the tools you need.

[Enable Task routing](/en/docs/task-routing/#delegation-skill) in Claude Code and try: “Use HiRoute to delegate an investigation of the configuration loading path to Pi code investigation. Return entry points and links to the evidence.” Check Pi's result in **Agent → Task history**. The delegation plan also works while Claude Code keeps its native model.

Next: [Add a model route to Pi](/en/docs/pi-model-routing/) · [See the full delegation workflow](/en/docs/task-routing/)
