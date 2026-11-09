# Codex model routing: combine your subscription and APIs

Keep working in Codex. Send code reading and small edits to an economical model, and keep your primary model for difficult debugging. Try a HiRoute route in an on-demand Codex CLI profile before enabling it for your everyday setup.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/codex-model-routing/ and reuse my Codex sources to set up an on-demand route. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find Codex sources and the configuration target: hiroute compute scan · compute connection options · quickstart
- Create a difficulty route and map Codex: routing preview / apply · agents connect preview / apply / status
- Choose another executor and check the run: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Preview the on-demand setup first. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

## 1. Find the model sources you already use in Codex

Open **Models → General models → Add models → Scan this device**. If the scan finds a Codex provider, check the connection and save the models you want. For a Codex subscription, reuse a supported file-based sign-in, or sign in independently under **Connect a subscription**. Use independent sign-in when Codex stores its credentials in the system keychain.

To add an economical model, use **New API connection**. You can also use [Qoder models through Magpie](/en/docs/qoder-model-routing/) or [a local model you already connected](/en/docs/models/).

## 2. Create a “Codex daily coding” route

Open **Smart routing → New smart routing** and select **Difficulty routing**. Pick a Regular model for file search, test summaries and routine edits. Keep a familiar Codex subscription or API model in Primary. Start with **Heuristic rules** or add a [decision model](/en/docs/model-routing/#difficulty-routing) for stage scoring and low-score protection. Publish the route.

Open **Agent → Codex → Model routing**. Choose **On demand · CLI only**, select the route and set it as the default for that profile. Launch Codex with the command from **Enable and copy launch command**. Ask it to find a module's test entry points, then give it a question that needs analysis across files. Open **Sessions → Routing timeline** to see which model ran.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-en.webp"><img src="/guide-assets/agent-editor-en.webp" alt="Codex model routing dialog with CLI access modes and available routes" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Try the on-demand CLI profile first. Your everyday Codex entry keeps its own settings.</figcaption>
</figure>

### Use the route in Codex Desktop

On macOS or Windows, select the separate **Codex Desktop** entry in HiRoute Desktop, save the route and choose **Launch separate window**. If you choose **Default · CLI and Desktop** instead, it affects the entries that share that configuration directory. Earlier sessions are filtered by provider; use **Find earlier sessions** to locate them.

## 3. Stay in Codex and let DSH do an independent analysis

Reuse your models in a plan named “DSH analysis” with its own Regular and Primary groups. Under **More settings → Task delegation**, allow delegation, describe the purpose, select **DeepSeek Harness**, check its dependencies and publish.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-dsh-worker-en.webp"><img src="/guide-assets/agent-guide-dsh-worker-en.webp" alt="DeepSeek Harness selected as the execution Agent for a HiRoute task" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Give the DSH plan a clear purpose so Codex can find it for the right work.</figcaption>
</figure>

[Enable Task routing](/en/docs/task-routing/#delegation-skill) in Codex. Try: “Use HiRoute to delegate analysis of this test failure to DSH analysis. Return reproduction steps and a minimal fix proposal.” See the result in **Agent → Task history**. Both plans can use the same models, and you can enable delegation while Codex keeps its original model settings.

Next: [Set up DeepSeek Harness](/en/docs/deepseek-harness-model-routing/) · [See other model sources](/en/docs/models/)
