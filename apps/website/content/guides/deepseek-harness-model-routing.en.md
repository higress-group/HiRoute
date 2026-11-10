# DeepSeek Harness model routing for Web and Desktop

Keep the DeepSeek Harness workflow. Use a fast model for straightforward questions and a primary model for deeper analysis. Add a route to DSH Web or Desktop, or have another Agent delegate work to DSH.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/deepseek-harness-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find DSH and importable models: hiroute quickstart · compute scan · compute connection prepare
- Publish a plan and connect the right entry: routing preview / apply · agents connect preview / apply
- Check executors and actual models: worker executors · worker result · sessions timeline
The UI and CLI share configuration and records. Distinguish Web from Desktop and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

Open **Models → General models → Add models → Scan this device** and find static providers from your DSH Web/CLI setup. Select a connection with explicit model declarations, check it and save the models you want.

To add a source, choose DeepSeek or Custom API under **New API connection**, enter its address and key, check it and save. You can also include imported subscription models in the same route.

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “DSH analysis” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

In **Agent**, select **DeepSeek Harness Web** or **DeepSeek Harness Desktop**, matching the client you use. Open **Model routing**, check “DSH analysis” and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-dsh-en.webp"><img src="/guide-assets/agent-guide-dsh-en.webp" alt="DeepSeek Harness Web model routing settings" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add the published route to the DSH client you use.</figcaption>
</figure>

Restart that client, select the route in its model picker and try a log-analysis or code-reading task. See the models used in HiRoute's **Sessions → Routing timeline**. Web and Desktop have separate route settings.

<h2 id="task-delegation">4. Explore task delegation</h2>

When analyzing a problem in DSH Web or CLI, hand a focused fix to Codex CLI. Another Agent can also delegate root-cause investigation to DSH.

Bind an executor in the plan under **More settings → Task delegation**. To delegate from DSH, enable **Task routing** in the **DeepSeek Harness Web** entry in HiRoute. [Continue with the Agent routing guide](/en/docs/task-routing/)
