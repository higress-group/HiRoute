# Connect models

HiRoute manages subscriptions, APIs and local models in one place on your machine. The Models page has two kinds: **General models** answer users, call tools and complete tasks; **Decision models** only judge task difficulty and assess work. See [Smart model routing](/en/docs/model-routing/) for decision models.

This guide covers general models: how to connect them, how to combine a subscription and an API that offer the same model into one route, and how to limit spending on an API key. Desktop and the Linux Web UI work the same way; for the matching `compute` commands, see [HiRoute CLI](/en/docs/cli/).

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for your Agent">
<div class="agent-guide-heading"><strong>Let your Agent use this guide</strong></div>

Copy these instructions into your Agent conversation and describe what you want to do. They include documentation links, built-in Skill setup and CLI pointers.

<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read the documentation below, then use the HiRoute CLI to carry out my goal for “Connect models.” Use the goal from our conversation and ask only for missing information you need.

Page documentation: https://hiroute.ai/en/docs/models/
CLI reference: https://hiroute.ai/en/docs/cli/

Use the built-in HiRoute management Skill. If it is not enabled, guide me through “Agent → HiRoute management → Enable”: https://hiroute.ai/en/docs/#management-skill

Command pointers (choose only what the goal needs; prefix each with hiroute):
- Inspect or reuse connected models: compute list · compute show · compute connection options
- Scan local settings, connect a subscription or API: compute scan · compute connection prepare / authorize / login / test
- Select models and save a connection: compute connection preview → compute connection apply
- Combine the same model from several sources: routing options → routing preview → routing apply
- Set key allowances in the connection details. Read or edit model prices with prices effective · prices override preview / apply.
- Set a connection proxy: compute network show / preview / apply

The UI and CLI connect to the same HiRoute service and share configuration and execution records. Inspect and reuse existing state. Get exact parameters from the installed command’s --help and hiroute schema show --command-id COMMAND_ID --include-schema. Follow the built-in Skill for the required preview, apply and verification, then report the result.
```

</details>
</aside>

## Add models

Open **Models → General models** and select **Add models**:

- **Connect subscription:** sign in to a Codex or Claude Code subscription independently in HiRoute. HiRoute refreshes this sign-in automatically, so the native client does not need to keep running.
- **Scan this device:** find local Agents, Codex / Claude Code subscriptions and model configurations, and reuse them. A reused subscription still depends on its native client to refresh the sign-in. Codex can reuse only file-based sign-ins; for Keychain or other stores, use an independent sign-in.
- **New API connection:** start from a template, enter an API key, read the service's model list and choose the models to add. Reading the list runs no paid inference, and the key stays on this device. If no template fits, choose **Custom API** and enter the URL and protocol yourself; proxies and local models are connected this way too.
- **Add models to an existing connection:** reuse saved endpoints and credentials to add more models from the same service.
- **Browse free models:** see each service's free conditions and how to get a key.

After saving, a model shows **Connection ready**. This means it meets the local conditions for routing; it does not mean upstream inference or tool calls have been verified. Confirm with a real request.

<figure>
  <a href="/guide-assets/models-add-en.webp"><img src="/guide-assets/models-add-en.webp" alt="Add models dialog offering subscription, device scan and API connection choices" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Use Connect subscription for a sign-in, Scan this device to reuse local configuration, or New API connection for an endpoint and key.</figcaption>
</figure>

## Choose what to do next

Have a Qoder or WorkBuddy account? Connect the models it offers through [headless Magpie](/en/docs/magpie-model-sources/). See [Agent integrations](/en/docs/agent-integrations/) for other sources and setup steps.

- **Reduce model costs in your daily Agent:** create a [model route](/en/docs/model-routing/) and map it to your Agent.
- **Try a new model first:** open New models on Home and follow the [trial flow](/en/docs/model-trial/).
- **Run work with another Agent:** combine execution models and an execution Agent in an [Agent route](/en/docs/task-routing/).

You are ready to configure a route when the saved model appears among its candidates. A successful real request is the next check for upstream inference and tool use.

<h2 id="subscription-api-credits">Connect a subscription and its API credits</h2>

After you connect and check a Codex or Claude subscription, the same form lets you enter the matching OpenAI or Anthropic API key, or choose an existing API connection. Leave it blank to use the subscription only. Choose models and save once, and the available models from both sides are imported together.

Eligible subscriptions can claim API credits. Use the key from the receiving organization and [set an allowance based on the credit balance and renewal date](#api-key-allowance). Follow the [Codex API credits guide](/en/docs/codex-model-routing/#api-credits) or the [Claude Code API credits guide](/en/docs/claude-code-model-routing/#api-credits) to claim credits and set the candidate order.

<h2 id="same-model-candidates">Combine same-named models into one route</h2>

Open a model's details and select **Create routing**. If other connections offer the same model, HiRoute asks: "Other connections offer this model. Add them as candidates too?"

- **Add together as candidates:** the current model comes first and the other connections provide fallback.
- **Use current model only:** the route contains only this model.

The new route uses **Fixed model** and tries candidates in order, skipping candidates that are temporarily unavailable. You can also append the model to an existing fixed route. Check the candidate order, then enable the route or publish your changes.

Your Agent selects one route name and HiRoute tries candidates in the order you set. Put the subscription first to use API credits when subscription quota is unavailable. To use credits close to expiry, put the API first, set a key allowance based on your remaining credits, and keep the subscription as backup.

<h2 id="api-key-allowance">Limit API key spending</h2>

In an API connection's details, open **API keys and allowances**, turn on **Limit API spending** for a key and choose an allowance:

- **One-time amount:** for a fixed budget.
- **Recurring allowance:** renews daily, weekly, monthly or yearly, with a renewal day, time and time zone. As the balance rule, either restore the amount each period without carryover, or accumulate up to a balance cap.
- **Share an existing allowance:** several keys draw from one allowance.

Leave **Initial available amount** blank to start with the full amount, or enter the remainder if some credits were already spent.

Allowances estimate ordinary inference spending through HiRoute. For known prices, HiRoute reserves funds before forwarding a request and settles the charge when the request finishes. Decision model calls and their connection tests do not use this local allowance, even if they reuse the same API key. The provider can still bill or limit both types of traffic. Set the applicable input, output and cache rates in the same currency under **Edit pricing**. A request with unknown prices can proceed and stays unpriced in the records, outside the spending total. When the allowance cannot cover a known charge, HiRoute tries another key or the next candidate.

## Set a proxy

Each API connection has its own **Connection network settings**. Under **Connection mode**, choose:

- **Direct (default)**
- **Service environment proxy:** use the proxy environment the HiRoute service started with.
- **Manual proxy:** enter an HTTP / HTTPS proxy URL and, optionally, addresses that bypass it. Local addresses always connect directly, and proxy URLs cannot contain a username or password.

Subsequent requests use the new settings immediately; requests in progress keep their route. Saving does not verify connectivity. Decision models that reuse the connection follow the same settings. The proxy address is resolved on the machine running HiRoute: in a remote Web UI, `127.0.0.1` refers to the server, not the computer running your browser.

The proxy for subscription connections is set separately under **Subscription connection proxy** on the Models page: automatic (use the launch proxy), direct, or manual. Saving restarts HiRoute.

## Model information and new-model recommendations

Official model information, such as context windows and output limits, and the recommendations under **New models** on Home are maintained and published by hiroute.ai separately from the app. The local service fetches validated updates automatically, without an app upgrade. Published routes keep the capabilities they were published with; edit the model or publish again to adopt new information.

Providers may update a model behind the same name, and a date in the model information does not identify the version a connection actually serves.
