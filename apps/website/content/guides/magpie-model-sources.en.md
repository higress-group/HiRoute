# Connect model sources to HiRoute with headless Magpie

Run Magpie in the background to bring the models your [Qoder](/en/docs/qoder-model-routing/) or [WorkBuddy](/en/docs/workbuddy-model-routing/) account offers into HiRoute. Magpie handles sign-in and serves the local model API. In HiRoute, choose the model groups, route by difficulty and bind an execution Agent.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/magpie-model-sources/ and help me connect the models from my signed-in Magpie instance to HiRoute. CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
- Find and reuse existing connections: hiroute compute list · compute scan · compute connection options
- Check and save the API: compute connection test · compute connection preview / apply
- Inspect models and set up routing: routing options · routing preview / apply
The UI and CLI share configuration and records. Let me sign in and enter credentials locally. Follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema.
```

</details>
</aside>

<h2 id="start-magpie">1. Start the background gateway</h2>

Follow the [Magpie setup guide](https://usemagpie.ai/docs/start) to install the CLI on the same host as the HiRoute service. On macOS or Linux with systemd, install and check the user service:

```sh
magpie daemon install --no-web
magpie daemon status
```

If the installed service is stopped, run `magpie daemon start`. For a quick trial, run `magpie serve` in a terminal. The default address is `http://127.0.0.1:3425`. Check the status output for yours. If you came from an Agent tutorial, return there to sign in to your account.

<h2 id="model-catalog">2. Read the model catalog</h2>

Once you've authorized your model source, read the model IDs from the local gateway:

```sh
curl -fsS http://127.0.0.1:3425/v1/models \
  -H 'Authorization: Bearer magpie'
```

Use your own address or gateway key if you changed either. Keep the prefix in each model ID, such as `qoder/dfmodel`, and choose models listed for your account. See the [Magpie community plugin catalog](https://github.com/magpie-community/plugins/blob/main/registry.json) for more sources.

<h2 id="save-api">3. Save the Magpie API in HiRoute</h2>

Open **Models → General models → Add models → New API connection**, then select **Custom API**:

| Field | What to enter |
| --- | --- |
| Connection name | For example, “Local Magpie” |
| API base URL | `http://127.0.0.1:3425/v1`, or your actual address |
| Protocol | Select an available API. Start with Responses for the Pi/Codex CLI examples here. If you also use Chat Completions, you can save a separate connection to the same address |
| API key | `magpie` for the default local gateway, or the key you configured |
| Models | Check the connection, then pick models from the catalog or enter their complete IDs |

If the catalog does not appear, set the model directory path to `/v1/models` in **Advanced settings** and check again. You can also enter IDs from the catalog with **Edit models manually**.

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-magpie-api-en.webp"><img src="/guide-assets/agent-guide-magpie-api-en.webp" alt="Local Magpie address and Responses protocol in HiRoute's Custom API form" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Add Magpie as a model connection, choose the protocol and models you can use, then save.</figcaption>
</figure>

If a supported Codex or Hermes configuration already points at Magpie, try **Scan this device** and import the matching connection after checking it. If the scan finds nothing, add the API directly. When configuring from a remote browser, `127.0.0.1` means the host running the HiRoute service.

Next, [set up Regular and Primary models](/en/docs/model-routing/) or [choose your Agent's walkthrough](/en/docs/agent-integrations/).
