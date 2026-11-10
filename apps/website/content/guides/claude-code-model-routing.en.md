# Claude Code subscription setup and model routing

Import your Claude subscription models into HiRoute and use a familiar Opus, Sonnet or Haiku preset in Claude Code to select the route. Give routine edits to an economical model and complex debugging to a primary model.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/claude-code-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find a subscription or API: hiroute compute scan · compute connection options / prepare
- Create the plan and preset mapping: routing preview / apply · agents connect preview / apply
- Configure an executor and check results: worker executors · worker plans · worker result · sessions timeline
The UI and CLI share configuration and records. Let me authorize the connection. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

Import the models from your Claude subscription into HiRoute.

1. Open **Models → General models → Add models → Connect subscription**.
2. Click **Sign in** on the **Claude Code** card. Authorize with your Claude account in the browser, then return to HiRoute.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-subscription-login-en.webp"><img src="/guide-assets/agent-guide-subscription-login-en.webp" alt="Claude Code sign-in entry in HiRoute's subscription dialog" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Click Sign in on the Claude Code card, authorize your account and return to HiRoute.</figcaption>
</figure>

3. Click **Check and choose models**, then **Check subscription** to load the models available to your account.
4. Select the models you want from your account's catalog, then click **Save and connect**. For example, pair Sonnet or Opus with an economical model you already use.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-subscription-models-en.webp"><img src="/guide-assets/agent-guide-claude-subscription-models-en.webp" alt="Selecting Sonnet and Opus from a checked Claude Code subscription" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose models returned for your account and save them to HiRoute.</figcaption>
</figure>

If you already signed in to Claude Code locally, use **Add models → Scan this device**, check the detected subscription and save its models. HiRoute can reuse file-based and macOS Keychain sign-ins. For an Anthropic API key, choose the service under **New API connection**, check it and save.

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Claude daily coding” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

Open **Agent → Claude Code → Model routing** and expand **Advanced settings**. Map the **Sonnet** preset to the “Claude daily coding” plan you published. You can also map Opus or Haiku to a route, then save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-en.webp"><img src="/guide-assets/agent-guide-claude-en.webp" alt="Claude Code Opus, Sonnet and Haiku presets mapped to HiRoute plans" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Map the Sonnet preset to your published difficulty route.</figcaption>
</figure>

Restart `claude`, use `/model` to select the mapped preset and send a small task. HiRoute chooses between the route's Regular and Primary groups. See the actual model choices in **Sessions → Routing timeline**.

<h2 id="task-delegation">4. Explore task delegation</h2>

While fixing a problem in Claude Code, ask Pi to trace configuration loading or independently review a diff. Keep working in the main conversation and incorporate its findings when it finishes.

To get started, bind an execution Agent under **More settings → Task delegation** in a plan and enable **Task routing** in the main Agent that sends the task. The executor uses the plan's models, and results appear in **Agent → Task history**. [Continue with the Agent routing guide](/en/docs/task-routing/)
