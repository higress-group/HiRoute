# Codex API credits and subscription routing

Connect your ChatGPT subscription and the OpenAI API organization that received your credits. With two sources for the same GPT model, Codex can use API credits when the subscription reaches its limit, or use credits that are close to expiring first. Then select the route in a separate Codex Desktop window or in Codex CLI.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/) · [Use your Codex subscription API credits](#api-credits)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/codex-model-routing/ and help me get models, configure smart routing and connect my Agent. Use this guide's lightweight Pi example to show how a scoped task can use fewer primary-model tokens. Enable delegation when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- If I have API credits, help me use the receiving organization's API connection and add the same model from both sources in my preferred order. See https://hiroute.ai/en/docs/codex-model-routing/#api-credits
- If I want a decision model for difficulty routing, reuse that OpenAI API connection. See decision connections options / list / apply / test and https://hiroute.ai/en/docs/codex-model-routing/#openai-decision-model
- Find Codex sources and the configuration target: hiroute compute scan · compute connection options · quickstart
- Connect the client I use, Codex Desktop or Codex CLI. Create the route and preview the connection with routing preview / apply · agents connect preview / apply / status.
- For Desktop, guide me to open the separate window from HiRoute. The agents desktop launch command also works in the local graphical session.
- For delegation, choose an executor and check the result with worker executors · worker plans · worker result.
The UI and CLI share configuration and records. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

[1. Scan or sign in to get models](#get-models) · [2. Configure smart routing](#configure-routing) · [3. Connect your Agent](#connect-agent) · [4. Explore task delegation](#task-delegation)

<h2 id="get-models">1. Scan or sign in to get models</h2>

With a ChatGPT subscription, sign in and choose the Codex models available to your account.

1. Open **Models → General models → Add models → Connect subscription**.
2. Click **Sign in** on the **Codex** card. Complete authorization with your ChatGPT account in the browser, then return to HiRoute.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-subscription-login-en.webp"><img src="/guide-assets/agent-guide-subscription-login-en.webp" alt="Codex and Claude Code sign-in buttons in HiRoute's Connect subscription dialog" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Click Sign in on the Codex card and authorize with your ChatGPT subscription account.</figcaption>
</figure>

3. When the card says **Signed in. Check and choose models**, click **Check and choose models**, then **Check subscription**. The available models appear when the check completes.
4. Select the models you want. To add credits as well, [enter the receiving organization's API key](#api-credits). Click **Save and connect** and confirm the models appear under **General models**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-subscription-models-en.webp"><img src="/guide-assets/agent-guide-codex-subscription-models-en.webp" alt="Selecting models from a checked Codex subscription before saving the connection" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Select the models available to your account, then click Save and connect to import them.</figcaption>
</figure>

For an existing Codex setup, **Add models → Scan this device** can find supported subscription or API settings. Check and save the connection. Codex sign-in reuse supports credentials stored in a file. For a system keychain sign-in, use the independent **Connect subscription** flow above. You can also add an API key through **New API connection**.

<h3 id="api-credits">Connect OpenAI API credits from your ChatGPT subscription</h3>

Eligible ChatGPT Pro and Business subscribers can claim OpenAI API credits. Link your subscription to the API organization receiving the credits, then create a project API key in that organization. Pay-as-you-go requests through that key can draw on the credits. Check eligibility, balance and expiry in the [official OpenAI API credits guide](https://developers.openai.com/api/docs/guides/chatgpt-api-credits).

1. Select the models in the checked subscription. Enter the receiving organization's key under **OpenAI API Key (optional)**, or choose a connection from that organization under **Use an existing API connection**.
2. Click **Save and connect**. HiRoute imports the subscription models and their supported exact-ID counterparts from the API. For a saved subscription, select it in **General models → Connections**, then choose **Associate an API connection** or **Edit association** under **Subscription and API**.
3. To budget API use around the credits, turn on **Limit API spending** during setup or later in the key's details. Use the provider's remaining credit amount and set a renewal schedule if the grant recurs. With model prices set, requests that exceed the local allowance can try the next available candidate. See [API key allowances](/en/docs/models/#api-key-allowance).

You can [combine subscription and API candidates for the same model](#credit-candidates) and [reuse the credit-funded connection for OpenAI Decisions](#openai-decision-model).

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Daily coding” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

<h3 id="openai-decision-model">Use API credits for an OpenAI decision model</h3>

Codex can keep using subscription models to run tasks while the OpenAI API connection from step 1 handles difficulty judgments and stage scoring.

1. Open the **Difficulty routing** plan you created above. Under **Decision method**, select **Decision model** and click **Add decision model**.
2. Under **Credentials**, select **Use existing API connection** and choose the OpenAI connection that uses your credits. HiRoute reuses the keys in that connection.
3. Set the model ID to `gpt-6-luna`, the complete URL to `https://api.openai.com/v1/decisions`, and the protocol to **OpenAI decisions**. Check any prefilled values. Name it “OpenAI difficulty judge” and click **Save and test**. [OpenAI Decisions documentation](https://developers.openai.com/api/docs/guides/decisions)
4. Return to the route, confirm the saved decision model is selected, and click **Publish changes**. After connecting Codex, try a small task and check its difficulty judgment and execution model under **Sessions → Routing timeline**.

Decision calls can draw on the API credits in the OpenAI organization linked to this connection. Even if an execution candidate uses the same key, decision calls do not consume HiRoute's [local key spending allowance](/en/docs/models/#api-key-allowance). [Learn about stage scoring and low-score protection](/en/docs/model-routing/#difficulty-routing)

<h3 id="credit-candidates">Add the subscription and API credits as same-model candidates</h3>

To use one model across the two sources, create a **Fixed model** route. Open **Models → General models** and switch to the **Models** view. Open the model from the source you want to use first. Click **Create routing**, then **Add together as candidates** in **Matching models found**. Check that the list contains the subscription and API sources you want. Set their order and enable the route. For an existing fixed route, new candidates are appended. Reorder them in the editor and publish the changes. Select the published route in Codex in the next step.

- **Subscription first.** Put the subscription first and let the API use credits when subscription quota is unavailable.
- **Credits first.** Put the API first when credits are close to expiring. Set a key allowance based on the credit balance so the subscription can take over when the allowance is too low.

For difficulty routing, use **Add model** to choose the subscription and API entries for the same model. Put both in the same Regular or Primary group and order the sources within that group.

<h2 id="connect-agent">3. Connect your Agent</h2>

Choose the [separate Codex Desktop window](#codex-desktop) or [Codex CLI](#codex-cli) based on the client you use.

<h3 id="codex-desktop">Use Codex Desktop</h3>

1. Install the ChatGPT / Codex desktop app on macOS or Windows, then open **HiRoute Desktop → Agent → Codex Desktop**.
2. Click **Choose routes and enable**. For an existing connection, click **Edit** beside **Model routing**. Check your published plan. A single route becomes the default automatically; if you choose several, pick your preferred route under **Default selection**. Click **Enable** or **Save settings**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-desktop-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-en.webp" alt="Codex Desktop model routing dialog with the Daily coding route selected" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Select your new route. When it is the only route, the separate window uses it by default.</figcaption>
</figure>

3. Back in the Codex Desktop details, click **Open separate window** in the **Separate window** section. HiRoute opens a desktop window configured to use your route.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-codex-desktop-launch-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-launch-en.webp" alt="Codex Desktop details in HiRoute with the Open separate window button" width="1200" height="900" loading="lazy" /></a>
  <figcaption>After saving the route, click Open separate window at the upper right and start work in the new window.</figcaption>
</figure>

4. Choose a project in the new window and click **New chat**. Confirm or choose your new route in the model picker at the bottom right of the composer. Try “Find this project’s startup entry points and explain how they connect.” In HiRoute, open **Sessions → Routing timeline** to see the models selected.

<figure class="guide-shot-window">
  <a href="/guide-assets/agent-guide-codex-desktop-window-en.webp"><img src="/guide-assets/agent-guide-codex-desktop-window-en.webp" alt="A new chat in the separate Codex Desktop window with Qoder Responses selected at the bottom right" width="2560" height="1640" loading="lazy" /></a>
  <figcaption>Confirm your route in the model picker at the bottom right. This example uses “Qoder Responses”. Click the screenshot to view it at full size.</figcaption>
</figure>

After changing route settings, quit this instance and open it again with the same button.

<h3 id="codex-cli">Use Codex CLI</h3>

Open **Agent → Codex → Model routing**. Confirm **On demand · CLI only** under **Advanced settings**, then check the route you published. If **Default selection** appears, choose your route. Click **Enable and copy launch command**, then run the command in a terminal in your project.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-en.webp"><img src="/guide-assets/agent-editor-en.webp" alt="On-demand Codex CLI access and default route settings" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose on-demand CLI access, then run the launch command provided by HiRoute.</figcaption>
</figure>

Try a small task and inspect the actual model choices in HiRoute's session timeline.

<h2 id="task-delegation">4. Explore task delegation</h2>

When CI fails, let lightweight Pi sift through the logs while Codex focuses on diagnosing and fixing the problem. Give Pi an economical model that supports tool calls. The cheaper model processes the logs and tool output, and the main chat gets a short summary.

Create a Fixed model route called “Pi log triage”. Under **More settings → Task delegation**, allow delegation and choose Pi, describe its purpose and enable the route. Turn on delegation under **Agent → Codex → Task routing**. If you use the separate Desktop window, send the task from [Codex CLI](#codex-cli).

> Use HiRoute to send this CI log to “Pi log triage”. Return the first failure, the relevant files and one command to reproduce it.

Ask Codex to retrieve the result. Confirm the executor in **Agent → Task history**, then check the model and usage in **Sessions → Routing timeline**. [Set up lightweight Pi delegation](/en/docs/task-routing/#lightweight-pi)
