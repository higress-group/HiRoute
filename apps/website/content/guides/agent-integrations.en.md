# Connect your Agent to HiRoute

Keep the Agent you work in and combine your subscriptions, APIs or local models in one route. Let an economical model read code and gather facts, bring in your primary model for harder turns, and hand independent work to another Agent when its tools suit the job.

[Download HiRoute](/en/download/) · [Get started with HiRoute](/en/docs/)

## Pick the Agent you use

Every guide follows the same four steps: get models, configure smart routing, connect your Agent, and explore task delegation. Routing instructions and screenshots are included on each page.

<ul class="integration-catalog">
  <li><a href="/en/docs/codex-model-routing/">Codex model routing and Desktop setup</a><p>Sign in to your subscription, configure a route and open a separate Codex Desktop window or CLI.</p></li>
  <li><a href="/en/docs/claude-code-model-routing/">Claude Code model routing</a><p>Map a route to an Opus, Sonnet or Haiku preset and send a research task to Pi.</p></li>
  <li><a href="/en/docs/qoder-model-routing/">Qoder CLI and subscription models</a><p>Discover models through Magpie, configure routing and use Qoder CLI or Codex Desktop.</p></li>
  <li><a href="/en/docs/pi-model-routing/">Pi model routing</a><p>Import models from Pi, select the route with <code>/model</code> and delegate verification to Codex CLI.</p></li>
  <li><a href="/en/docs/deepseek-harness-model-routing/">DeepSeek Harness model routing</a><p>Pick a route in Web or Desktop, or let DeepSeek Harness execute a delegated task.</p></li>
  <li><a href="/en/docs/opencode-model-routing/">OpenCode model routing</a><p>Reuse existing API settings, add a route to <code>/models</code> and ask Pi for an independent check.</p></li>
  <li><a href="/en/docs/hermes-model-routing/">Hermes Agent model routing</a><p>Reuse models from your active Hermes profile and hand coding tasks to Codex CLI.</p></li>
</ul>

## Connect more model sources

<ul class="integration-catalog">
  <li><a href="/en/docs/workbuddy-model-routing/">WorkBuddy models in Pi or Codex CLI</a><p>Bring in your WorkBuddy catalog and set up a route for document review.</p></li>
  <li><a href="/en/docs/magpie-model-sources/">Headless Magpie model sources</a><p>Start the local gateway, read the catalog and connect it to HiRoute.</p></li>
</ul>

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/agent-integrations/ and choose a guide based on my existing model sources and the Agent I use. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Read existing connections: hiroute quickstart · compute list · compute scan
- Publish a route and connect an Agent: routing preview / apply · agents connect preview / apply
- Check the actual model and executor: sessions timeline · worker plans · worker result
The UI and CLI share configuration and records. Reuse existing connections and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## From model sources to an execution Agent

1. **Connect models.** In **Models → General models → Add models**, connect a subscription, scan supported configurations on the device or add an API. If Magpie is already running, [add its local API](/en/docs/magpie-model-sources/#save-api).
2. **Route by difficulty.** Choose **Difficulty routing** under **Smart routing** and add Regular and Primary models. Start with heuristic rules if you want local decisions. Add a decision model for stage scoring and low-score protection. See [model routing](/en/docs/model-routing/#difficulty-routing).
3. **Choose how to use the route.** Map the published route to your Agent's model selector for everyday work. To hand off an independent task, bind an executor in a plan and enable **Task routing** in the Agent you talk to. You can use both paths or enable either one by itself.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="Regular and Primary model groups in a HiRoute difficulty routing plan" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Give routine work to Regular models and hard turns to Primary models. You can add several candidates to each group.</figcaption>
</figure>

Start with a small task. See which models ran in **Sessions → Routing timeline**, and see the executor and outcome in **Agent → Task history**. [Follow the delegation walkthrough](/en/docs/task-routing/)
