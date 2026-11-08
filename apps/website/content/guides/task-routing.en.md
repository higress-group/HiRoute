# Smart Agent routing

Stay in your usual Agent and hand work to a better-fitting **model × harness**. A harness is the Agent’s execution tool: it shapes how the Agent uses tools, manages context and completes work. HiRoute calls Agent routing “Task routing” in the UI. It chooses an execution Agent; model routing chooses the models used during execution.

| Your main Agent | Capability you want | Example task route |
| --- | --- | --- |
| Codex | Run a DeepSeek model with DeepSeek Harness (DSH) | “DSH development”: connect an available DeepSeek model and choose DeepSeek Harness as executor |
| Hermes | Use Codex for coding and tests | “Codex coding”: choose execution models and Codex CLI as executor |

The capabilities work independently. Your main Agent can keep its existing model and sign-in, delegating only when needed. The steps below show Desktop; the Linux Web UI works the same way.

**Use task routing through the built-in delegation Skill.** Enable it under “Agent → Task routing” as shown below, then give your main Agent the task goal. The Skill handles route discovery, delegation and result retrieval. An Agent with the [management Skill](/en/docs/#management-skill) can also configure routes; the UI and CLI share configuration and task records.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for your Agent">
<div class="agent-guide-heading"><strong>Let your Agent use this guide</strong></div>

Copy these instructions into your Agent conversation and describe what you want to do. They include documentation links, built-in Skill setup and CLI pointers.

<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read the documentation below, then use the HiRoute CLI to carry out my goal for “Smart Agent routing.” Use the goal from our conversation and ask only for missing information you need.

Page documentation: https://hiroute.ai/en/docs/task-routing/
CLI reference: https://hiroute.ai/en/docs/cli/

Use the built-in HiRoute management Skill. If it is not enabled, guide me through “Agent → HiRoute management → Enable”: https://hiroute.ai/en/docs/#management-skill
For task delegation, use the built-in delegation Skill. If needed, guide me through “Agent → Task routing → Enable” and follow the installed Skill: https://hiroute.ai/en/docs/task-routing/#delegation-skill

Command pointers (choose only what the goal needs; prefix each with hiroute):
- Choose an executor and allow delegation: worker executors · worker plans · routing preview + --delegation enable / --worker-harness / --worker-protocol → routing apply
- Enable the built-in delegation Skill: agents connect preview --context CONTEXT_ID --collaboration explicit → agents connect apply
- Let the built-in Skill start and follow tasks: worker exec · worker status · worker wait · worker result
- Read task history, continue or cancel: worker list · worker read · worker continue · worker cancel

Omit --plan when enabling only delegation to preserve the main Agent’s model connection. Select executor and protocol together. Follow the built-in delegation Skill to execute, follow up and recover tasks.

The UI and CLI connect to the same HiRoute service and share configuration and execution records. Inspect and reuse existing state. Get exact parameters from the installed command’s --help and hiroute schema show --command-id COMMAND_ID --include-schema. Follow the built-in Skill for the required preview, apply and verification, then report the result.
```

</details>
</aside>

## Before you start

Install the execution Agent on the HiRoute service host and [connect its models](/en/docs/models/). A Fixed model route is enough to start; you can later combine it with [Difficulty routing](/en/docs/model-routing/).

## What to delegate

Delegation works best for independently executable work with a clear goal, working directory, and deliverable—for example, implementing a bounded feature, investigating a group of test failures, or producing a research summary. The main agent discovers allowed plans, selects a fitting scenario, and retrieves the result.

Do not delegate coordination, plan discovery, status checks, waits, or result summaries. An execution agent cannot recursively create another delegation layer.

## 1. Configure a route for delegation

1. Open Smart routing and create or edit a plan.
2. Expand More settings, then under Task delegation turn on Allow delegation to an execution agent.
3. Enter a purpose. A purpose is required once delegation is on: describe the tasks that fit this route and the expected result, such as “Make well-scoped code changes and run the relevant tests.” A main agent chooses routes by their name and purpose.
4. Choose one execution agent: Codex CLI, Claude Code, Qoder CLI, Pi, DeepSeek Harness or OpenCode. Availability depends on the local installation and dependency checks. A plan has one executor and does not automatically switch to another agent on failure.
5. Check the required execution environment when prompted, then publish the plan.

A missing installation does not block saving the plan, but the chosen execution agent must pass its dependency check before a task can start.

<figure>
  <a href="/guide-assets/routing-worker-en.webp"><img src="/guide-assets/routing-worker-en.webp" alt="Route editor showing task delegation and execution Agent choices" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose the execution Agent in the route. This example uses Codex CLI; select DeepSeek Harness here for a DSH route.</figcaption>
</figure>

<h2 id="delegation-skill">2. Enable the built-in delegation Skill for your main Agent</h2>

Open Agent, choose the main agent you use every day, and select Enable under Task routing. Codex, Claude Code, Qoder, Pi, DeepSeek Harness, Hermes and OpenCode can all act as main agents; for Hermes, first select the configuration directory it actually uses.

1. Choose **When to delegate**. With **Only when explicitly requested**, the agent starts a task only when you ask it to execute, delegate, or hand work to a Worker. With **Let the Agent decide by default**, the agent may delegate suitable work on its own, and you can still ask it to answer without execution.
2. Select **Enable task routing** and complete the task delegation skill check when prompted.

Task routing affects only delegation; it does not change the model the main agent uses. A main agent can keep its own models and sign-in without model routing and still use HiRoute plans through task routing.

Manage routes available for task delegation under Smart routing. The main agent discovers published plans that allow task delegation.

<figure>
  <a href="/guide-assets/agent-task-en.webp"><img src="/guide-assets/agent-task-en.webp" alt="Codex Task routing dialog with delegation trigger choices" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Start with “Only when explicitly requested.” An already enabled connection shows “Save settings.”</figcaption>
</figure>

## 3. Delegate in your normal conversation

First check that the route is published, allows delegation and has a ready execution Agent. Replace the route names and absolute paths below with your own:

> In Codex: Use HiRoute to delegate the parser regression in `/absolute/path/to/project` to “DSH development.” Find the failure, fix it and run relevant tests. Summarize the changes and test results.

> In Hermes: Use HiRoute to delegate the login form in `/absolute/path/to/project` to “Codex coding.” Follow the existing components, implement it and run relevant tests. Summarize the result.

You can also use a general request:

- Choose a route: “Use HiRoute to delegate this task to [route name].” Replace the placeholder with a route that allows task delegation under Smart routing.
- Let the Agent choose: “Use HiRoute to delegate this task and choose a suitable route.”

With Let the Agent decide by default, the main agent may also select a plan for suitable independent work. Accepted work appears under **Task history** on the Agent page.

The built-in delegation Skill uses the `worker` commands below. In everyday use, describe the task to your main Agent.

## 4. Follow progress and retrieve the result

Open “Agent → Task history” and select the task. Check its route, execution Agent and run state. Ask the main Agent to retrieve the result, then inspect the actual changes and test output. Saved configuration or an accepted submission does not mean the task has finished.

<figure>
  <a href="/guide-assets/tasks-en.webp"><img src="/guide-assets/tasks-en.webp" alt="Task history showing the task list, execution Agent and run state" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Example task history: follow execution here, then have your main Agent retrieve and summarize the result.</figcaption>
</figure>

Add instructions by continuing the same task. If it fails, inspect the cause before continuing or starting again. If a connection drops, query the original submission as shown below to avoid duplicate work.

## CLI execution and recovery

<details>
<summary>Show commands, submission identifiers and connection recovery</summary>

After installing the [HiRoute CLI](/en/docs/cli/), discover executors and plans:

```sh
hiroute worker executors
hiroute worker plans
```

Then start a task:

```sh
hiroute worker exec \
  --plan <PLAN_ID> \
  --cwd /absolute/path/to/project \
  --title "Fix the parser regression" \
  --submission-key parser-fix-001 \
  -- "Find the failure, implement the smallest fix, and run the relevant tests"
```

After submitting, `worker exec` waits up to 30 seconds for the task's next progress, then returns the current state and run ID; if the task has already succeeded, the result is included. The task keeps running in the background regardless of this wait. Use `--wait-timeout` to adjust the wait (1–30 seconds), or `--no-wait` to return right after submission. `--run-timeout` limits the task's own run time; it defaults to 1 hour, up to 24 hours.

`--submission-key` identifies this submission; use a new value for a new task. Keep it before submitting. After acceptance, also save the returned task ID and run ID for status, continuation or cancellation.

If the connection drops before you know whether the task was accepted, query the original submission key:

```sh
hiroute worker status --submission parser-fix-001 --operation start
```

Do not retry with a new key, which could start a duplicate task.

While the task runs, check it with `hiroute worker status --run <RUN_ID>`. When it ends, read its result with `hiroute worker result --run <RUN_ID>`, and use `hiroute worker continue` to add instructions to the same task. See [HiRoute CLI](/en/docs/cli/) for the full commands.

</details>

## Permission and cancellation boundaries

Delegated tasks run with the `approve-all` permission policy, without per-tool approval: the selected execution agent can use its native read, write, command, and network behavior. It is not an operating-system sandbox, and `--cwd` is the task's working directory, not a directory authorization. Delegate only where these actions are acceptable, and prepare isolation such as a container or a separate account yourself when you need it.

Cancellation stops the selected run; it does not undo files or external effects already produced. Concurrent tasks are not serialized merely because their working directories overlap. The caller decides whether overlapping execution is appropriate.
