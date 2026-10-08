# Use smart task routing

Model routing decides “which model should handle this stage.” Task routing decides “which execution agent should own this work.” You can enable either capability independently or combine them.

The sections below begin with the Desktop configuration path. Linux headless can publish the same plan through `routing preview/apply`, connect an Agent through `agents connect preview/apply`, and then use the same Worker commands. Obtain exact fields from the installed schema.

## What to delegate

Delegation works best for independently executable work with a clear goal, working directory, and deliverable—for example, implementing a bounded feature, investigating a group of test failures, or producing a research summary. The main agent discovers allowed plans, selects a fitting scenario, and retrieves the result.

Do not delegate coordination, plan discovery, status checks, waits, or result summaries. An execution agent cannot recursively create another delegation layer.

## Enable delegation on a plan

1. Open Smart routing and create or edit a plan.
2. Under Task delegation, turn on Allow delegation to an execution agent.
3. Choose one execution agent for the plan. Current options are Codex CLI, Claude Code, Qoder CLI, Pi and DeepSeek Harness; availability depends on the local installation and dependency checks. A plan has one executor and does not automatically switch to another agent on failure.
4. Check the required execution environment when prompted, then publish the plan.

A missing installation does not block saving the plan, but the chosen execution agent must pass its dependency check before a task can start. Give the plan a specific name and purpose that describe fitting tasks and expected results, so a main agent can choose it correctly.

## Allow a main agent to delegate tasks

Open Agents, choose your main agent with task-routing support, then open Task routing:

1. Enable task routing.
2. Choose when to delegate: only when you explicitly ask, or let the Agent decide by default.
3. Save the settings and complete the task delegation skill check when prompted.

Manage routes available for task delegation under Smart routing. The main agent discovers published plans that allow task delegation.

After enabling, explicitly mention HiRoute and delegation in the agent's normal conversation, for example:

- Choose a route: “Use HiRoute to delegate this task to [route name].” Replace the placeholder with a route that allows task delegation under Smart routing.
- Let the Agent choose: “Use HiRoute to delegate this task and choose a suitable route.”

If you choose the default-delegation behavior, the main agent may also select a plan for suitable independent work. Desktop lists accepted work on the Tasks page.

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

`--submission-key` identifies this submission; use a new value for a new task. Keep it before submitting. After acceptance, also save the returned task ID and run ID for status, continuation or cancellation.

If the connection drops before you know whether the task was accepted, query the original submission key:

```sh
hiroute worker status --submission parser-fix-001 --operation start
```

Do not retry with a new key, which could start a duplicate task. See [HiRoute CLI](/en/docs/cli/) for progress and result commands.

## Permission and cancellation boundaries

The default `approve-all` lets the selected harness use its native read, write, command, and network behavior. It is not an operating-system sandbox. If you need a restriction, select only a policy that the page or CLI says the harness supports. An unsupported restriction fails before the prompt instead of silently widening access.

Cancellation stops the selected run; it does not undo files or external effects already produced. Concurrent tasks are not serialized merely because their working directories overlap. The caller decides whether overlapping execution is appropriate.
