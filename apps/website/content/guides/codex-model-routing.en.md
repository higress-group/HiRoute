# Codex subscription model routing and Desktop setup

Connect your ChatGPT subscription and API models to HiRoute. Use an economical model for reading code and small changes, and a primary model for harder debugging. Follow the screenshots to import models, configure difficulty routing and use it in a separate Codex Desktop window or Codex CLI.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/codex-model-routing/ and help me get models, configure smart routing and connect my Agent. Explain when to use task delegation and enable it when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- Find Codex sources and the configuration target: hiroute compute scan · compute connection options · quickstart
- Create a difficulty route and map Codex: routing preview / apply · agents connect preview / apply / status
- Choose another executor and check the run: worker executors · worker plans · worker result
The UI and CLI share configuration and records. Preview the on-demand setup first. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
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
4. Select the models you want and click **Save and connect**. Confirm they appear under **General models**, ready to add to your route.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-codex-subscription-models-en.webp"><img src="/guide-assets/agent-guide-codex-subscription-models-en.webp" alt="Selecting models from a checked Codex subscription before saving the connection" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Select the models available to your account, then click Save and connect to import them.</figcaption>
</figure>

For an existing Codex setup, **Add models → Scan this device** can find supported subscription or API settings. Check and save the connection. Codex sign-in reuse supports credentials stored in a file. For a system keychain sign-in, use the independent **Connect subscription** flow above. You can also add an API key through **New API connection**.

<h2 id="configure-routing">2. Configure smart routing</h2>

1. Open **Smart routing → New smart routing**, name the plan “Daily coding” and choose **Difficulty routing**.
2. Click **Add model** in **Regular models** and choose a fast or economical model from step 1 for reading files, finding definitions and gathering facts.
3. Add a model to **Primary models** for complex analysis, planning or changes across several files. Combine available models from different subscriptions, APIs or local connections as needed.
4. Start with **Heuristic rules** under **Decision method**, then click **Enable**. The published route becomes available in the Agent settings in the next step.

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-en.webp"><img src="/guide-assets/agent-guide-difficulty-en.webp" alt="HiRoute difficulty routing with a decision method and Regular and Primary model groups" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Regular models handle routine work and Primary models handle harder turns. Replace the illustrated models with the ones you connected in step 1.</figcaption>
</figure>

To adjust routing using execution feedback, select a decision model in the same section for stage scoring and low-score protection. [Learn about difficulty decisions](/en/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. Connect your Agent</h2>

### Use Codex Desktop

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
  <figcaption>In the separate window, confirm the route in the bottom-right model picker. This example uses “Qoder Responses”; choose the route you created.</figcaption>
</figure>

After changing route settings, quit this instance and open it again with the same button.

### Use Codex CLI

Open **Agent → Codex CLI → Model routing**. Choose **On demand · CLI only**, check the route you published and make it the default. Click **Enable and copy launch command**, then run the copied command in a terminal in your project.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-editor-en.webp"><img src="/guide-assets/agent-editor-en.webp" alt="On-demand Codex CLI access and default route settings" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose on-demand CLI access, then run the launch command provided by HiRoute.</figcaption>
</figure>

Try a small task and inspect the actual model choices in HiRoute's session timeline.

<h2 id="task-delegation">4. Explore task delegation</h2>

While coding, you may want another Agent to investigate an error or check test coverage. Codex CLI can hand that scoped task to DSH or Pi and bring the result back to your conversation.

To get started, bind an execution Agent under **More settings → Task delegation** in a plan and enable **Task routing** in the main Agent that sends the task. The executor uses the plan's models, and results appear in **Agent → Task history**. [Continue with the Agent routing guide](/en/docs/task-routing/)
