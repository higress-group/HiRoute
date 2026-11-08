# Quickstart

HiRoute helps you choose **which model handles a stage and which Agent handles a task.** Start with either path, or combine them.

| What you want | Where to start | What you get |
| --- | --- | --- |
| Lower everyday coding costs and try cheaper models | **Model routing**: connect models → create a route → connect your usual Agent | Regular models for simple work, a primary model for harder stages, and records of usage and performance |
| Use another model and execution tool from your usual Agent | **Agent routing**: configure an execution Agent → enable task routing → delegate | Call DeepSeek Harness from Codex, or hand coding work from Hermes to Codex |

Screenshots use dark mode and example data. Select an image to open it at full size.


<details class="agent-prompt" id="agent-cli">
<summary>Ask an Agent: set up model routing (CLI)</summary>

Copy this into an Agent that can run commands on the target host, replace the bracketed values, then send it.

```text
Use the HiRoute CLI to set up model routing for me.
Target Agent: [name; specify a directory or profile if there are several]
Model source: [subscription / saved API connection / provider and endpoint to add]
Regular model: [name]; Primary model: [name]
Route name: [Daily development]; Purpose: [my typical tasks]
Decision method: [saved decision connection / local heuristic rules]
Verification task: [optional; blank means configure only, with no model call]

1. Run hiroute quickstart, hiroute compute list and hiroute routing list. Reuse resources and resolve the exact target context and model bindings. Give Hermes an explicit --hermes-root.
2. If a source is missing, use compute connection options, prepare or protected authorization as appropriate; preview --apply-file, then apply --request-file.
3. Read routing options. Create a smart_saving route with routing preview, --economy, --primary, and either --decision local_rules or --decision-service ID@REVISION. Generate a fresh Apply file and publish it.
4. Prepare agents connect preview --context CONTEXT_ID --plan PLAN_ID. For Codex, prefer a separate on-demand CLI profile and explicitly choose native model retention and default policies. Follow installed help for other Agents' protocols or presets. Change only model routing.
5. After applying, query agents connect status --effective and follow its native loading instructions. Run only the verification task supplied above, then match session evidence to the route, model and result. Saved configuration does not prove effective use.

Execution rules: inspect current configuration, command --help and the required schemas; use returned IDs, revisions and protocols. Preview first, check that the effects match this goal, then Apply. Ask only for missing required choices or changes beyond the goal. Keep private Apply files and submission identifiers; look up an uncertain operation before retrying the same submission. Credentials use protected input in my own terminal, never chat, arguments or ordinary files. Report saved configuration, native settings still to load, and real execution evidence or remaining verification separately.
```

See [HiRoute CLI](/en/docs/cli/) for command formats, credential setup and recovery.

</details>

<details class="agent-prompt">
<summary>Ask an Agent: set up cross-Agent delegation (CLI)</summary>

Copy this into an Agent that can run commands on the target host, replace the bracketed values, then send it.

```text
Use the HiRoute CLI to configure cross-Agent delegation.
Main Agent: [Codex / Hermes / another; specify directory or profile]
Execution Agent: [DeepSeek Harness / Codex CLI / another]
Execution model: [name and source]; Task route name: [name]
Purpose: [suitable tasks and deliverables]
Configure only for now; do not start a task. Keep the main Agent's existing model connection.

1. Run hiroute quickstart, hiroute worker executors and hiroute worker plans. Resolve the main Agent's context, executor readiness and existing routes. Connect missing models through compute connection first.
2. Create or edit a routing preview with the execution model, --delegation enable, and both --worker-harness and --worker-protocol. Supply the purpose. Codex CLI uses codex_cli; DeepSeek Harness uses deepseek_harness. Select the protocol from current requirements.
3. After previewing and publishing, use agents connect preview --context CONTEXT_ID --collaboration explicit --apply-file NEW_FILE to enable explicit-request delegation only. Omit --plan here; preserve the main Agent's model and management Skill. Complete any required local skill check before applying.
4. Confirm the route is discoverable in worker plans, the executor is ready and task routing is configured. Give me a delegation prompt using the exact route name, and list any missing prerequisites.

Execution rules: inspect current configuration, command --help and the required schemas; use returned IDs, revisions and protocols. Preview first, check that the effects match this goal, then Apply. Ask only for missing required choices or changes beyond the goal. Keep private Apply files and submission identifiers; look up an uncertain operation before retrying the same submission. Credentials use protected input in my own terminal, never chat, arguments or ordinary files. Report saved configuration, native settings still to load, and real execution evidence or remaining verification separately.
```

See [HiRoute CLI](/en/docs/cli/) for command formats, credential setup and recovery.

</details>

## Install and start HiRoute

- **macOS**: requires macOS 15.0 or later. Public packages are self-signed. Verify the SHA256, then follow [Install and open on macOS](/en/docs/install-macos/).
- **Windows**: native x64, installed for the current user, with the service and CLI included. No Python or administrator rights required. See [Windows installation](/en/docs/install-windows/).
- **Linux**: x86_64 and ARM64, without `sudo`. Enable the Web UI for the same interface in your browser, or use the CLI alone. See [Linux installation](/en/docs/install-linux/).

The [download page](/en/download/) lists released packages. Check that the service is ready before continuing. The browser manages models, Agents and files on the service host. Keep the HiRoute service running while using routes or executing tasks.

## Path one: use model routing in your usual Agent

Start here to address model costs or try a new model on real work.

1. **Connect models.** Open “Models → General models → Add models.” Connect a subscription, scan local configuration or add an API. Check the connection, select models and save. See [Connect models](/en/docs/models/).
2. **Create a route.** Open “Smart routing → New smart routing.” Choose “Fixed model” to get a first connection working. For simple work on a cheaper model, choose “Difficulty routing,” configure Regular and Primary groups and a decision method, then select “Enable.”
3. **Connect your usual Agent.** Open “Agent,” select Codex or another target, and choose “Enable” under “Model routing.” Select the published route and save. With Codex, you can choose “On demand · CLI only” and copy the launch command. For other modes, restart or load configuration as the page instructs.
4. **Run a small task and check.** Select the route in your Agent and run a task with a clear expected result. In “Sessions,” confirm the actual route, model and usage. Saving configuration does not switch an already running session.

**Want to try a new model directly?** Use “Try” or “Connect and try” in Home’s “New models” section. The flow takes you through model selection, route publication and Agent mapping. The new model starts with simple turns, backed by your familiar primary model.

<figure>
  <a href="/guide-assets/trial-entry-en.webp"><img src="/guide-assets/trial-entry-en.webp" alt="Dark-mode Home with a new-model card and trial action" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Start a trial from a new-model card on Home. Whether you need a connection depends on the models you already have.</figcaption>
</figure>

Continue with [Try a new model](/en/docs/model-trial/) or [Configure model routing](/en/docs/model-routing/). Actual savings depend on prices, task mix and fallback frequency; use your own execution records to assess them.

## Path two: delegate work to another Agent

Your daily conversation does not have to use the same model and harness (the Agent’s execution tool) as every task. Stay in Codex and delegate an independent task to DeepSeek Harness using a DeepSeek model, or ask Hermes to hand coding work to Codex.

1. **Prepare the execution Agent.** Install it on the HiRoute service host and connect the model it will use.
2. **Publish a route for delegation.** Create or edit a route under “Smart routing,” choose its models, then expand “More settings → Task delegation.” Turn on “Allow delegation to an execution agent,” describe its purpose, choose an execution Agent, check its environment and publish.
3. **Enable task routing for your main Agent.** Under “Agent,” select the Agent you use daily and enable “Task routing.” Start with “Only when explicitly requested,” complete the skill check shown on the page and save. Your main Agent can keep its existing model connection.
4. **Delegate explicitly and follow the result.** Ask: “Use HiRoute to delegate this task to [route name], then summarize the changes and test results.” Replace the placeholder with a published route name and specify the goal and working directory. Follow progress under “Agent → Task history.”

<figure>
  <a href="/guide-assets/agent-task-en.webp"><img src="/guide-assets/agent-task-en.webp" alt="Main Agent task-routing settings with a choice of delegation trigger" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Model routing and task routing are configured separately. This setting controls when the main Agent delegates.</figcaption>
</figure>

Delegated tasks run with `approve-all`: the execution Agent can write files, run commands and access the network. The working directory is not a sandbox. Start with a small, well-scoped task in an environment where those operations are acceptable.

Continue with [Agent routing: setup, examples and results](/en/docs/task-routing/).

## Configure from the CLI or an Agent

Run `hiroute quickstart` to inspect service status, exact configuration targets and reusable sources/routes. Once chosen, add `--context CONTEXT_ID --plan PLAN_ID`. This entry point reads status and explains what is still missing. To target Hermes, add `--hermes-root /ABSOLUTE/EFFECTIVE/ROOT`.

The [HiRoute CLI guide](/en/docs/cli/) covers previews, applying configuration, task execution and recovery. The same management Skill can manage another target from any supported Agent while the caller keeps its native model connection. Enter credentials through a hidden prompt in your own terminal, never in chat.
