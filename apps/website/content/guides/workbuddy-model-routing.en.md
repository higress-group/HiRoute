# Use WorkBuddy models in Pi with headless Magpie and HiRoute

Bring your WorkBuddy models into HiRoute. Route straightforward work to a Regular model and harder decisions to a Primary model, then use the route in Pi or Codex CLI. WorkBuddy supplies the models while your preferred Agent reads files, edits code and organizes research.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/workbuddy-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Inspect and save an API connection: hiroute compute list · compute connection options / test / preview / apply
- Configure smart routing and connect Pi: routing options · routing preview / apply · agents connect preview / apply
- Inspect model choices; choose an executor when delegation is requested: sessions timeline · worker executors · worker result
The UI and CLI share configuration and records. Let me authorize WorkBuddy and use IDs from my catalog. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#connect-workbuddy) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="connect-workbuddy">1. Scan or sign in to get models</h2>

Sign in to WorkBuddy through headless Magpie, read your account's model catalog and connect it to HiRoute. [Install the Magpie CLI](https://usemagpie.ai/docs/start), then run these commands. With an existing background service, go straight to installing the plugin and signing in.

```sh
magpie daemon install --no-web
magpie plugin add @magpie-community/opencode-workbuddy-auth
magpie plugin login workbuddy
curl -fsS http://127.0.0.1:3425/v1/models \
  -H 'Authorization: Bearer magpie'
```

Use `workbuddy` for the China build with a CodeBuddy plan. For the international build, use `magpie plugin login workbuddy-ai`. After authorizing in the browser, query the catalog again and keep the complete IDs of the models you want.

In HiRoute, open **Models → General models → Add models → New API connection → Custom API** and fill in the connection.

| Setting | Value |
| --- | --- |
| API base URL | `http://127.0.0.1:3425/v1`, or your gateway's address |
| Protocol | Responses for the Pi or Codex CLI examples below |
| API key | `magpie` for the default local gateway, or your configured key |
| Models | Select from the returned catalog, keeping the full source prefix |

Click **Check and continue**, select models and save. If the catalog does not appear, set the model directory path to `/v1/models` under **Advanced settings**, or enter the complete IDs you just retrieved. If a supported Agent configuration already points to this gateway, try **Scan this device** to find and reuse the connection.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-en.webp"><img src="/guide-assets/agent-guide-magpie-api-en.webp" alt="Magpie API address and Responses protocol in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Save the Magpie API that supplies your WorkBuddy models.</figcaption>
</figure>

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “WorkBuddy document review” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

### Use WorkBuddy models in Pi

Open **Agent → Pi → Model routing** and check the “WorkBuddy document review” plan you published. Choose Responses to match the connection and save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-workbuddy-pi-en.webp"><img src="/guide-assets/agent-guide-workbuddy-pi-en.webp" alt="Pi model routing selection in HiRoute" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Select your newly created WorkBuddy route in Pi's connection settings.</figcaption>
</figure>

Restart Pi and choose the route with `/model`. Ask it to extract three takeaways from the project README. The models come from your WorkBuddy connection, and Pi handles actions such as reading files. Inspect the model choices in HiRoute's **Sessions → Routing timeline**.

You can also select the plan under **Agent → Codex CLI → Model routing** and launch it with the command from **On demand · CLI only**. See the [illustrated Codex setup](/en/docs/codex-model-routing/#connect-agent).

<h2 id="task-delegation">4. Explore task delegation</h2>

When organizing research, ask Pi to read documents and Codex CLI to check their code examples, then combine the results in your main conversation. WorkBuddy supplies the models, and you choose an execution Agent to suit each task.

To get started, bind an execution Agent under **More settings → Task delegation** in a plan and enable **Task routing** in the main Agent that sends the task. The executor uses the plan's models, and results appear in **Agent → Task history**. [Continue with the Agent routing guide](/en/docs/task-routing/)
