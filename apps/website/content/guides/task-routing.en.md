# Use smart task routing

Model routing decides “which model should handle this stage.” Task routing decides “which execution agent should own this work.” You can enable either capability independently or combine them.

The sections below show Desktop; the Linux Web UI has the same steps. You can also use only the CLI: publish the plan through `routing preview/apply`, connect an Agent through `agents connect preview/apply`, and then use the Worker commands below. Obtain exact fields from the installed schema.

## What to delegate

Delegation works best for independently executable work with a clear goal, working directory, and deliverable—for example, implementing a bounded feature, investigating a group of test failures, or producing a research summary. The main agent discovers allowed plans, selects a fitting scenario, and retrieves the result.

Do not delegate coordination, plan discovery, status checks, waits, or result summaries. An execution agent cannot recursively create another delegation layer.

## Enable delegation on a plan

1. Open Smart routing and create or edit a plan.
2. Expand More settings, then under Task delegation turn on Allow delegation to an execution agent.
3. Enter a purpose. A purpose is required once delegation is on: describe the tasks that fit this route and the expected result, such as “Make well-scoped code changes and run the relevant tests.” A main agent chooses routes by their name and purpose.
4. Choose one execution agent: Codex CLI, Claude Code, Qoder CLI, Pi, DeepSeek Harness or OpenCode. Availability depends on the local installation and dependency checks. A plan has one executor and does not automatically switch to another agent on failure.
5. Check the required execution environment when prompted, then publish the plan.

A missing installation does not block saving the plan, but the chosen execution agent must pass its dependency check before a task can start.

## Allow a main agent to delegate tasks

Open Agent, choose the main agent you use every day, and select Enable under Task routing. Codex, Claude Code, Qoder, Pi, DeepSeek Harness, Hermes and OpenCode can all act as main agents; for Hermes, first select the configuration directory it actually uses.

1. Choose **When to delegate**. With **Only when explicitly requested**, the agent starts a task only when you ask it to execute, delegate, or hand work to a Worker. With **Let the Agent decide by default**, the agent may delegate suitable work on its own, and you can still ask it to answer without execution.
2. Select **Enable task routing** and complete the task delegation skill check when prompted.

Task routing affects only delegation; it does not change the model the main agent uses. A main agent can keep its own models and sign-in without model routing and still use HiRoute plans through task routing.

Manage routes available for task delegation under Smart routing. The main agent discovers published plans that allow task delegation.

After enabling, explicitly mention HiRoute and delegation in the agent's normal conversation, for example:

- Choose a route: “Use HiRoute to delegate this task to [route name].” Replace the placeholder with a route that allows task delegation under Smart routing.
- Let the Agent choose: “Use HiRoute to delegate this task and choose a suitable route.”

With Let the Agent decide by default, the main agent may also select a plan for suitable independent work. Accepted work appears under **Task history** on the Agent page.

Any other agent that can run commands can delegate directly with the `worker` commands below.

## Start a task from Terminal

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

## Permission and cancellation boundaries

Delegated tasks run with the `approve-all` permission policy, without per-tool approval: the selected execution agent can use its native read, write, command, and network behavior. It is not an operating-system sandbox, and `--cwd` is the task's working directory, not a directory authorization. Delegate only where these actions are acceptable, and prepare isolation such as a container or a separate account yourself when you need it.

Cancellation stops the selected run; it does not undo files or external effects already produced. Concurrent tasks are not serialized merely because their working directories overlap. The caller decides whether overlapping execution is appropriate.
