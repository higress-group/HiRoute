# Connect models

HiRoute manages subscriptions, APIs and local models in one place on your machine. The Models page has two kinds: **General models** answer users, call tools and complete tasks; **Decision models** only judge task difficulty and assess work. See [Smart model routing](/en/docs/model-routing/) for decision models.

This guide covers general models: how to connect them, how to combine a subscription and an API that offer the same model into one route, and how to limit spending on an API key. Desktop and the Linux Web UI work the same way; for the matching `compute` commands, see [HiRoute CLI](/en/docs/cli/).

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

- **Reduce model costs in your daily Agent:** create a [model route](/en/docs/model-routing/) and map it to your Agent.
- **Try a new model first:** open New models on Home and follow the [trial flow](/en/docs/model-trial/).
- **Run work with another Agent:** combine execution models and an execution Agent in an [Agent route](/en/docs/task-routing/).

You are ready to configure a route when the saved model appears among its candidates. A successful real request is the next check for upstream inference and tool use.

## Connect a subscription and its API together

After you connect and check a Codex or Claude subscription, the same form lets you enter the matching OpenAI or Anthropic API key, or choose an existing API connection. Leave it blank to use the subscription only. Choose models and save once, and the available models from both sides are imported together.

Some subscriptions include API credits granted each billing cycle. Enter the key of the organization that receives the credits, then give that key a spending allowance as described below, and the credits can serve requests through HiRoute.

## Combine same-named models into one route

Open a model's details and select **Create routing**. If other connections offer the same model, HiRoute asks: "Other connections offer this model. Add them as candidates too?"

- **Add together as candidates:** the current model comes first and the other connections provide fallback.
- **Use current model only:** the route contains only this model.

The new route uses **Fixed model** and tries candidates in order, skipping candidates that are temporarily unavailable. You can also add the model to an existing route; existing candidates keep their order and new models are appended. Nothing takes effect until you save or publish the route.

The subscription and the API then act as one model: your Agent uses a single model name, HiRoute uses the subscription first, and when the subscription is temporarily unavailable it continues with the API.

## Limit API key spending

In an API connection's details, open **API keys and allowances**, turn on **Limit API spending** for a key and choose an allowance:

- **One-time amount:** for a fixed budget.
- **Recurring allowance:** renews daily, weekly, monthly or yearly, with a renewal day, time and time zone. As the balance rule, either restore the amount each period without carryover, or accumulate up to a balance cap.
- **Share an existing allowance:** several keys draw from one allowance.

Leave **Initial available amount** blank to start with the full amount, or enter the remainder if some credits were already spent.

Allowances estimate HiRoute traffic only; they are not the provider's account balance. HiRoute reserves funds before forwarding a request and settles them when it finishes. The estimate needs the model's prices: complete all four rates in the same currency under the model's **Edit pricing**. When prices are missing or the allowance is insufficient, HiRoute uses another key or the next candidate.

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
