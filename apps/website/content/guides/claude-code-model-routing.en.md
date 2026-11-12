# Claude Code API credits and subscription routing

Connect your Claude subscription and the Claude API organization that received your credits. Put both sources for the same model in one route so the subscription and API credits can back each other up. Select the route through a familiar Opus, Sonnet or Haiku preset in Claude Code.

[Download HiRoute](/en/download/) · [Browse Agent guides](/en/docs/agent-integrations/) · [Use your Claude Code subscription API credits](#api-credits)

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/claude-code-model-routing/ and help me get models, configure smart routing and connect my Agent. Use this guide's lightweight Pi example to show how a scoped task can use fewer primary-model tokens. Enable delegation when I request it. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
For delegation, use the built-in delegation Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill
- If I have API credits, help me use the receiving organization's API connection and add the same model from both sources in my preferred order. See https://hiroute.ai/en/docs/claude-code-model-routing/#api-credits
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
4. Select models from your account's catalog, such as Sonnet or Opus. To add credits as well, [enter the receiving organization's API key](#api-credits). Click **Save and connect**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-subscription-models-en.webp"><img src="/guide-assets/agent-guide-claude-subscription-models-en.webp" alt="Selecting Sonnet and Opus from a checked Claude Code subscription" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose models returned for your account and save them to HiRoute.</figcaption>
</figure>

If you already signed in to Claude Code locally, use **Add models → Scan this device**, check the detected subscription and save its models. HiRoute can reuse file-based and macOS Keychain sign-ins. For an Anthropic API key, choose the service under **New API connection**, check it and save.

<h3 id="api-credits">Connect Claude API credits from your subscription</h3>

Eligible Claude Max, Ultra and Team subscribers can claim Claude API credits. Link the credits to a Claude Console organization and create an API key there. When Claude Code uses the API candidate through HiRoute, the Console organization applies credits to the Claude API requests. Check eligibility, balance and expiry in the [official Claude API credits guide](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers).

1. Select the models in the checked subscription. Enter the receiving organization's key under **Anthropic API Key (optional)**, or choose a connection from that organization under **Use an existing API connection**.
2. Click **Save and connect**. HiRoute imports the subscription models and their supported exact-ID counterparts from the API. For a saved subscription, select it in **General models → Connections**, then choose **Associate an API connection** or **Edit association** under **Subscription and API**.
3. To budget API use around the credits, turn on **Limit API spending** during setup or later in the key's details. Use the provider's remaining credit amount and set a renewal schedule if the grant recurs. With model prices set, requests that exceed the local allowance can try the next available candidate. See [API key allowances](/en/docs/models/#api-key-allowance).

Next, [put the subscription and API credit sources in one route](#credit-candidates).

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

<h3 id="credit-candidates">Add the subscription and API credits as same-model candidates</h3>

To use one model across the two sources, create a **Fixed model** route. Open **Models → General models** and switch to the **Models** view. Open the model from the source you want to use first. Click **Create routing**, then **Add together as candidates** in **Matching models found**. Check that the list contains the subscription and API sources you want. Set their order and enable the route. For an existing fixed route, new candidates are appended. Reorder them in the editor and publish the changes. Map the published route to a Claude Code preset in the next step.

- **Subscription first.** Put the subscription first and let the API use credits when subscription quota is unavailable.
- **Credits first.** Put the API first when credits are close to expiring. Set a key allowance based on the credit balance so the subscription can take over when the allowance is too low.

For difficulty routing, use **Add model** to choose the subscription and API entries for the same model. Put both in the same Regular or Primary group and order the sources within that group.

<h2 id="connect-agent">3. Connect your Agent</h2>

Open **Agent → Claude Code → Model routing** and expand **Advanced settings**. Map the **Sonnet** preset to the route you published. You can also map Opus or Haiku to a route, then save.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-en.webp"><img src="/guide-assets/agent-guide-claude-en.webp" alt="Claude Code Opus, Sonnet and Haiku presets mapped to HiRoute plans" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Map the Sonnet preset to your published route.</figcaption>
</figure>

Restart `claude`, use `/model` to select the mapped preset and send a small task. HiRoute selects the model and source using your route. See the actual model choices in **Sessions → Routing timeline**.

<h2 id="task-delegation">4. Explore task delegation</h2>

When reviewing a large change, give lightweight Pi the first pass over the diff and tests. An economical model can read the files and identify coverage gaps. Claude Code gets the short findings and decides which gaps deserve new tests.

Create a Fixed model route called “Pi coverage gaps”. Under **More settings → Task delegation**, allow delegation and choose Pi, describe the tasks it should take and enable the route. Turn on **Task routing** for **Claude Code** in HiRoute.

> Use HiRoute to delegate the coverage check for this diff to “Pi coverage gaps”. Return at most three findings with file locations and suggested cases.

Ask Claude Code to retrieve the findings and decide which tests to add. [Configure Pi delegation and check its usage](/en/docs/task-routing/#lightweight-pi)
