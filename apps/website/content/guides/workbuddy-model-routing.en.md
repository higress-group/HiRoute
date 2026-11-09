# Use WorkBuddy models in Pi with headless Magpie and HiRoute

Bring the models your WorkBuddy account offers into HiRoute. Route straightforward work to a Regular model and harder decisions to a Primary model, then run the task with Pi or Codex CLI. WorkBuddy supplies the models. The plan picks the execution Agent.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/workbuddy-model-routing/ and use my WorkBuddy catalog through Magpie to set up difficulty routing with the executor I choose. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Inspect and save an API connection: hiroute compute list · compute connection options / test / preview / apply
- Configure routing and the executor: routing options · routing preview / apply · worker executors
- Check the task and actual models: worker result · sessions timeline
The UI and CLI share configuration and records. Let me authorize WorkBuddy and use IDs from my catalog. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

<h2 id="connect-workbuddy">1. Find WorkBuddy models through Magpie</h2>

[Start the headless Magpie service](/en/docs/magpie-model-sources/#start-magpie), or use your running instance. Then install the [WorkBuddy community plugin](https://github.com/magpie-community/plugins/blob/main/registry.json) and sign in:

```sh
magpie plugin add @magpie-community/opencode-workbuddy-auth
magpie plugin login workbuddy
```

Use `workbuddy` for the China build with a CodeBuddy plan. For the international build, run `magpie plugin login workbuddy-ai`. After authorization, [read the model catalog](/en/docs/magpie-model-sources/#model-catalog) and keep the complete IDs available to your account.

[Save the Magpie API in HiRoute](/en/docs/magpie-model-sources/#save-api) with the WorkBuddy models you want to use. If you already have a connection to that Magpie endpoint, add the models to it and continue to the routing step.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-en.webp"><img src="/guide-assets/agent-guide-magpie-api-en.webp" alt="The local Magpie API address for connecting WorkBuddy models in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add the local Magpie address in HiRoute and pick models from your account's catalog.</figcaption>
</figure>

## 2. Set up a route for reading and analysis

Create a plan named “WorkBuddy document review” and choose **Difficulty routing**. Pick a Regular model for finding files and extracting facts, and a Primary model for comparing options and drawing conclusions. If your account offers just one suitable model, fill the other group from an [API, subscription or local connection](/en/docs/models/).

Start with **Heuristic rules**, or add a decision model to [route using stage scores](/en/docs/model-routing/#difficulty-routing). Choose models that support the tools your task needs, then publish.

## 3. Execute with Pi or Codex CLI

Open **More settings → Task delegation** in the plan. Allow delegation, describe the purpose (“Read project documentation and extract key points”), choose **Pi**, check its dependencies and publish. For a task that needs Codex's tools, reuse the same models in a separate plan and choose **Codex CLI** as executor.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-pi-worker-en.webp"><img src="/guide-assets/agent-guide-pi-worker-en.webp" alt="A HiRoute delegation plan with Pi bound as its execution Agent" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose the tool that runs the task here. The model groups above govern routing.</figcaption>
</figure>

[Enable Task routing](/en/docs/task-routing/#delegation-skill) in the Agent you usually talk to. Try: “Use HiRoute to delegate a review of this project's README to WorkBuddy document review. Return three takeaways.” Check **Agent → Task history** for the result and the session's routing timeline for model choices.

Next: [Connect Qoder models through Magpie](/en/docs/qoder-model-routing/) · [Add a route to Pi's model picker](/en/docs/pi-model-routing/)
