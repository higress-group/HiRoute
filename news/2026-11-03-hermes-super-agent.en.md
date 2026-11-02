# A DeepSeek-Style Super Agent with Hermes and HiRoute

On October 31 the DeepSeek Harness team published a long article, "Exploring Super-agent Design: Towards a More Autonomous Agent System", and open-sourced a research prototype, [dsh-superagent-prototype](https://github.com/deepseek-ai/dsh-superagent-prototype). Its starting point resonated with me: one person can follow only three to five active Agent sessions at a time. As models get more capable, human attention becomes the bottleneck.

I use Hermes as my main Agent every day. It is an open-source personal agent framework that you can run from a chat app. Independent tasks I delegate through [HiRoute](https://github.com/higress-group/HiRoute) to different execution Agents. Looking at DeepSeek's design, this combination already is a Super Agent: Hermes is the Router, and HiRoute's routing plans and execution Agents are the Workers. This article covers how it fits together, where it matches DeepSeek's prototype and where it differs, and the boundaries to know before you use it.

## What DeepSeek proposed

The article describes a two-layer Router and Worker architecture:

- **The Router** is resident. It handles outside communication, overall coordination and dispatch, and need not execute tasks at all. In the public prototype the Router does not even get a Bash tool.
- **Workers** each focus on a single task in isolated context and return their results to the Router, which replies on their behalf.

With this split, the Router can always respond to new messages instead of getting stuck behind a long task, and each Worker sees only its own context, so tasks do not interfere with each other.

To be clear about what it is: dsh-superagent-prototype is a research prototype, not an official DeepSeek product. It works on one GitHub repository and can also listen to Feishu. When someone asks it for work, it opens a work item, runs it in its own session and directory, and answers in the original conversation, for example with a pull request. Later comments on the same work item go straight to the matching Worker through a subscription. The prototype's README also notes that some designs in the article are future directions that the prototype does not implement.

## Three jobs for a Router

Putting this design into my own daily work, I find that a Router has at least three jobs:

1. **Stay unblocked.** After dispatching a task, the Router must get straight back to the conversation, and when the task ends, someone has to bring the result back.
2. **Pick who does the work.** Drafting, coding, correctness review and restructuring each suit a different execution Agent.
3. **Pick how expensive a model to use.** Within one task, most steps are routine and only a few need a strong model.

DeepSeek's article focuses on the first job. The prototype creates a Worker for each new work item, and the model a Worker uses depends on the DeepSeek Harness profile you configure. The other two jobs are exactly what HiRoute does.

![Super Agent architecture: Hermes acts as the Router, understands intent and picks a plan, submits with hiroute worker exec --no-wait and returns to the chat at once; each HiRoute routing plan binds one execution Agent: research goes to Pi, engineering to Qoder CLI, correctness review to Codex CLI and structure review to DeepSeek Harness; when a run ends, Hermes wakes, fetches the result and replies; inside each plan, System One picks the economy or primary group turn by turn](assets/super-agent-en.svg)

### 1. Stay unblocked: return on submit, wake on completion

Hermes hands a task to HiRoute with a command like this:

```sh
hiroute worker exec --plan <PLAN_ID> --cwd /work/mq-research --no-wait \
  -- "Collect the official docs and release notes of three message queues and build a comparison table"
```

With `--no-wait`, the command returns a task ID and a run ID as soon as the task is accepted, and Hermes goes straight back to the conversation. Several tasks can go out at once, each running in its own session.

The result comes back through Hermes's built-in `notify_on_complete`. Hermes can run a command in a background process, and with `notify_on_complete` turned on, Hermes wakes up and starts a new turn when that process ends. The approach is to have Hermes put "wait for this task to finish" into such a background process. When the task ends, Hermes wakes up, fetches the result with `hiroute worker result --run <RUN_ID>` and tells you in the conversation. The wait happens in the background and never occupies the conversation.

Why not use Hermes's own subagents? Hermes's `delegate_task` can also run work in the background, but its documentation is explicit: top-level delegation is asynchronous yet still tied to the owning session and the Hermes process, so closing the session, `/stop`, `/new` or a process restart can cancel or strand work in progress. For work that must survive those boundaries, the documentation recommends scheduled jobs or a background process with `notify_on_complete`. HiRoute tasks run in HiRoute's background service, their results are stored, and you can look them up by run ID at any time.

### 2. Pick who does the work: one plan, one execution Agent

In HiRoute, each routing plan covers one kind of work and states three things: what the work is (its purpose); which execution Agent does it, one of Codex CLI, Claude Code, Qoder CLI, Pi, DeepSeek Harness or OpenCode; and how models are tiered into an economy group and a primary group.

Before delegating, Hermes runs `hiroute worker plans`, reads the published plans that allow delegation and picks one by the purpose each plan states. If none fits, it says so instead of forcing a choice. You can also name a plan directly: "Use HiRoute to delegate this task to [Research]."

Do not write a purpose as a vague word like "development" or "work". I spell out four things: what tasks it suits, what input and working directory it needs, what it delivers, and how it differs from neighboring plans. "Review a pull request and post comments back" and "Review a local diff and list the problems" should be two separate plans.

From my own use, the four execution Agents split the work roughly like this:

| Execution Agent | What I hand it |
| --- | --- |
| Pi | Collecting material, first drafts and batch changes from a checklist; with a clear brief it runs fast |
| Qoder CLI | Engineering code; boundary wording and Chinese prose in public documents |
| Codex CLI | Correctness review: checks each command against the installed help and schema to confirm it runs and where its boundaries are; slower and more expensive, so I save it for work where mistakes are costly |
| DeepSeek Harness | Narrative and structure: finds where readers drop off and proposes a new layout; not a substitute for fact-by-fact checks |

This is my experience, not an evaluation result. Your tasks may call for a different split, and changing the execution Agent only means editing a plan, not touching Hermes.

### 3. Pick the model cost: another layer of routing inside each execution Agent

Choosing a plan only decides who does the work. The model behind each of that Agent's turns is decided by the plan's model routing. At the start of each turn, System One answers three questions: how did the last step go, how hard is this step, and will the last step's problem carry into this one. Simple-enough work goes to the economy group. When the last step fell short and the problem carries forward, the primary model takes over, and once the problem is resolved, work returns to the economy group. Tool calls within a turn never switch models, so the cache is not wasted.

For a delegated task, the first turn starts with the task itself; with no previous step, it is judged by difficulty alone. When Hermes adds a request to the same task with `hiroute worker continue`, that is a new turn, judged again with the previous turn's score.

So "who does the work" and "how expensive a model it uses" are two independent decisions: choosing Qoder CLI does not lock in a model tier. Over time, each plan's model performance report summarizes how well the economy group performs and, with enough evidence, suggests raising or lowering trust. A suggestion is only a suggestion, and nothing is published until you replay and confirm it.

## A real example: the previous version of this article

This article had a longer earlier version about my full delegation setup in Hermes. That version went from a 2.7 KB brief to a push to the repository with exactly this division of work:

| Stage | Execution Agent | Deliverable |
| --- | --- | --- |
| First draft (17 minutes) | Pi | 2.7 KB brief to a 298-line draft, self-checked and pushed |
| Correctness review | Codex CLI | Checked every command against the installed schema |
| Narrative review (6 minutes) | DeepSeek Harness | Diagnosed a "manual-like" feel and proposed a scenario-first structure |
| Boundary and Chinese review | Qoder CLI | Tightened boundary statements and prose |
| Second pass (9 minutes) | Pi | Revised with all three reviews and cut the body by 29% |
| Restructure | DeepSeek Harness | Rebuilt the outline scenario-first |
| Finish (1 minute) | Pi | Front matter, trailing spaces, `git diff --check` |

Only Pi's and DeepSeek Harness's stages were timed; the Codex and Qoder reviews were not timed separately. DeepSeek Harness, incidentally, served as a Worker twice in this workflow.

I made sure the three reviews could not see each other's conclusions. They agreed on the big picture: too manual-like, boundaries need to be explicit, commands need to run. Their unique findings barely overlapped:

- **Codex was the only one to check whether each command runs.** Comparing against the installed `--help` and schema, it caught a nonexistent `--timezone` option in a scheduled-job example and eight commands missing `--non-interactive`.
- **DeepSeek Harness's diagnosis was closest to how readers feel.** It concluded that the manual tone drowned out the message, and proposed the scenario-first structure in six minutes.
- **Qoder was the most precise about what the setup can and cannot do**, and wrote the best Chinese prose.

Were two extra reviews worth it? If the three had only echoed each other, they would have been a waste. They saw different things, and that is what makes the division of work pay off. It is also the point of a Super Agent: the Router does not need to be good at everything, only to know what to hand to whom.

## How to set it up

The full steps are in [smart task routing](https://hiroute.ai/en/docs/task-routing/). Here are the three key ones:

1. **Create plans.** Under **Smart routing** in HiRoute, create a plan for each kind of work, turn on **Allow delegation to an execution agent**, choose the execution Agent, write a clear purpose and publish. The execution Agent must pass its local dependency check before a task can start.
2. **Let Hermes delegate.** On the **Agent** page, choose Hermes, open **Task routing** and choose when to delegate: only when you explicitly ask, or let the Agent decide by default. Save, then complete the task delegation skill check when prompted. You can also route Hermes's own model requests through HiRoute; the two settings are independent.
3. **Delegate from Hermes.** Tell Hermes "Use HiRoute to delegate this task to [plan name]" or "Use HiRoute to delegate this task and choose a suitable route". Then add a standing instruction for Hermes: after delegating, put the wait in a background process with `notify_on_complete`, and reply once the result is fetched.

Other personal agents such as OpenClaw can call the same `hiroute worker` commands directly, as long as they can run a local command line.

## Boundaries to know first

This combination follows the same idea as DeepSeek's prototype but differs in a few ways:

- **A non-executing Router is a convention, not isolation.** DeepSeek's prototype simply gives the Router no Bash; Hermes keeps its own tools. If you let the Agent decide by default, it tends to delegate work that can run independently, but it does not lose the ability to execute. For hard isolation, restrict tools in Hermes's own configuration.
- **Follow-ups go through the Router.** In the prototype, later comments on a work item go straight to its Worker. Here, an execution Agent cannot ask you questions mid-task; Hermes sends any follow-up to the same task with `hiroute worker continue`.
- **Delegation is one level deep.** An execution Agent cannot delegate further. Plan discovery, status checks, waiting and summaries all stay with Hermes.
- **One plan binds one execution Agent.** If a run fails, HiRoute does not switch to another kind of Agent.
- **Execution permissions are not a sandbox.** Execution Agents run with approve-all and can read and write files, run commands and access the network as far as their own capabilities allow. HiRoute provides no operating-system sandbox, so do not delegate work that must be strictly read-only. A working directory is not a lock either; if several tasks share one, keep them from overwriting each other.
- **Cancelling stops only that run.** Files already written and external actions already completed are not undone.

## Closing thoughts

DeepSeek splits a Super Agent into a Router and Workers, and I agree with that direction. In daily work, though, you do not have to build one from scratch. A personal agent you already chat with, plus a layer of task routing, keeps the Router responsive and hands each task to a suitable execution Agent, while the model behind each of that Agent's turns is still chosen from evidence, one turn at a time.

HiRoute and Hermes are both open source. Give it a try, and tell us where it falls short.

[GitHub](https://github.com/higress-group/HiRoute) · [Smart task routing](https://hiroute.ai/en/docs/task-routing/) · [New HiRoute release](https://hiroute.ai/en/news/new-model-trials-and-task-routing/)
