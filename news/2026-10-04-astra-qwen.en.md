# HiRoute Smart Routing in Action: GPT-6 Astra × Qwen-3.8 Flash, Same Quality at 90% Lower Cost

**Does every step of a software iteration need the most expensive model?**

Reading documentation, organizing evidence, implementing changes, running tests, and diagnosing difficult failures can all happen within one agent task. They demand different capabilities. Some require careful coverage of explicit information; others require deriving guarantees or spotting subtle failure windows. A strong model throughout can overprice the former. An economical model throughout can miss the latter.

We put **GPT-6 Astra + Qwen-3.8 Flash** through two kinds of practical work. In two engineering-research pairs meeting the same delivery standard, mixed routing reduced **API-equivalent cost by 92.01% and 91.39%**. In a separate real coding task, an agent **received no intermediate operator guidance and passed 343 independent assertions after automatic model handoff**. The first case tests cost-effective allocation; the second observes autonomous escalation and completion.

## What HiRoute is, and where it fits

**HiRoute is an open-source intelligent routing engine that runs locally between existing agents and model services, designed for long-running tasks.** You continue assigning work in your familiar agent. That agent reads files, writes code, executes tools, and runs tests. HiRoute receives its model requests, selects the actual model according to a configured routing plan, and records calls and execution performance.

Connect model sources in HiRoute, then create a plan. Each plan has a stable “connection model name”: the agent uses that name to call HiRoute, while the route behind it can contain one model or a combination such as Qwen and Astra. The agent's connection stays consistent while routing decisions can change the model doing the work.

For example, this case puts Qwen in the economy group and Astra in the primary group. Explicit source verification can go to Qwen; analysis of cross-system consistency guarantees can go to Astra. At a context handoff during a long task, evidence that economy is not sufficiently capable can send subsequent requests to primary.

Desktop manages models, plans, and session inspection. The local Gateway handles model requests, and the CLI supports automation. For multi-agent work, a plan can also specify an execution agent to which the primary agent delegates by purpose. The two experiments here focus on model routing and do not depend on multi-agent delegation.

[![HiRoute connects agents to model services through routing plans and uses execution evidence to inform later choices](assets/long-horizon-engine-en.png)](assets/long-horizon-engine-en.png)

Open any figure to view it at full size.

## One plan, different capability needs as work progresses

A software iteration does not simply become harder over time. Solving an architecture problem may leave a large amount of routine implementation. Nearly finished code can still stall on a concurrency issue or edge case. Model allocation needs to accommodate that progression.

| Situation | How routing participates |
| --- | --- |
| Extract, verify, and organize information from supplied sources | Use economy when decision criteria permit, reducing the cost of bulk work |
| Derive guarantees, reconcile contradictions, or analyze complex failures | Select primary for stronger reasoning |
| Continue calling tools within the same segment of work | Inherit the existing route to keep execution stable |
| Rebuild context, or receive a user follow-up with reselection enabled | Reassess the task and consider available evidence from the preceding stage |
| A source is unavailable or a request fails | Try candidates within the plan's capability and fallback rules; a response already being delivered does not transparently change models midway |

Candidate fallback after a failed request addresses availability. Escalation based on performance addresses whether the selected capability is sufficient. The long-task experiment examines the latter.

## A cheaper model can create a more expensive task

When an economical model completes most of the work and a stronger model handles a few difficult decisions, mixed routing can save substantially. But if the economical model repeatedly takes the wrong path, the stronger model may need to reconstruct context, diagnose mistakes, and redo the implementation. You have paid for the first attempt and for the cleanup.

Extra attempts, growing context, and rework can consume the unit-price advantage. The relevant accounting is:

> **Whole-task cost = productive execution + failed attempts and rework + routing evaluation.**

HiRoute therefore distinguishes two questions:

| Assessment | Question | Role in routing |
| --- | --- | --- |
| Task complexity | What capability does the next piece of work require? | Identify suitable opportunities to use economy |
| Stage competence | How well did the selected model actually handle the preceding work? | Stop pursuing savings when observed performance is inadequate |

The competence assessment used in this case considers answers and tool activity visible to the routing layer, failed and recovered attempts, and available user feedback. It looks back at an assessable stage of execution. Useful progress with an appropriate process and no material correction differs from repeated errors and limited progress.

**The score concerns a particular stage. It is neither a permanent model ranking nor a probability of success.** Read it alongside the model, the work assessed, and evidence coverage. Scores for different stages are not a same-task model comparison. A missing assessment is not a zero; independent acceptance still determines whether the final delivery meets requirements.

[![Task complexity and observed competence inform the next choice at a decision opportunity](assets/competence-feedback-en.png)](assets/competence-feedback-en.png)

## Switch at context handoffs without watching every step

An agent's ongoing work accumulates conversation, tool results, and code changes. As context approaches its limit, the agent typically compacts history and continues from a summary. This is a common **context handoff** in long tasks.

HiRoute checks continuity between requests. Ordinary tool continuations inherit the route. When the existing context can no longer be inherited and a fresh decision is needed, it considers the current task and preceding stage performance. The client does not need to send a separate compaction notification.

This experiment sets a competence floor of **0.5**. At a decision opportunity, a stage score below that floor blocks the economy branch and selects primary. Suitable work and sufficient performance can also keep the same model in use.

That makes “try a stronger model” an automatic part of execution, without always waiting for the user to notice a problem and intervene. Assessment happens at decision opportunities, not after every tool call, and compaction does not automatically require an upgrade. Different models cannot share a KV cache; switching still builds a new context prefix.

## How to configure this setup in HiRoute

The product setup has three steps: make the model combination active, then check how it actually executes.

1. **Connect sources on the Models page.** Confirm that Qwen-3.8 Flash and GPT-6 Astra are available for routing through your normally authorized model services.
2. **Create a plan on the Smart routing page.** Choose “Smart saving,” place Qwen in “Economy group” and Astra in “Primary group,” and set reasoning effort to `xhigh` and `medium`, respectively. Confirm the “Connection model name” and enable the plan. Later edits take effect after publishing changes.
3. **Point the client at the plan.** When managing an agent through Desktop, select its model route on the Agent page. This coding experiment used isolated configuration to connect native Codex to the mixed plan; the research case used a separate exported client configuration for each plan to submit the same tasks through HiRoute. Check actual models, request records, and usage on the Sessions page.

For the cost comparison, create two additional “Fixed model” plans: one containing only Astra, the other only Qwen. All three modes receive the same frozen sources, verification obligations, and delivery requirements. Mixed routing does not get an easier task.

The model groups and experimental decision parameters are below. Thresholds are part of the frozen experiment policy; the experiment directory contains the full configuration and rerun instructions.

| Setting | Research cost experiment | Long-task handoff experiment |
| --- | --- | --- |
| Economy group | Qwen-3.8 Flash, xhigh | Qwen-3.8 Flash, xhigh |
| Primary group | GPT-6 Astra, medium | GPT-6 Astra, medium |
| Simple-task threshold | 0.8 | 0, economy-first start |
| Competence floor | 0.5 | 0.5 |
| Context window / maximum output | 131,072 / 65,536 tokens | 131,072 / 65,536 tokens |
| Outcome of interest | Whole-task cost at the same acceptance standard | Autonomous escalation and completion without intermediate guidance |

The starting policies serve different purposes. Research reserves primary for consequential judgments; the long task observes whether execution starting with economy can hand off based on performance. The long-task client also uses a **60,000-token** automatic compaction threshold and continues after one initial instruction. These settings are not universal optima.

To reproduce the setup, use the [research case](../experiments/cases/research-cost-quality/README.md) to prepare frozen sources and separate configurations for the three plans, or the [long-task case](../experiments/cases/unattended-engineering/README.md) to prepare pinned HTTPX. Keep fresh results separate from the historical records published here.

## Experiment one: 360 research cards and one critical decision

The workload represents preparation for a messaging-system architecture review. It covers RabbitMQ, Kafka, NATS, Pulsar, Redis Streams, and RocketMQ, with 60 distinct verification obligations per system. Each of the 360 cards needs a verdict, a factual explanation, and source-line citations.

A separate memo examines Kafka and SQL transaction semantics: safety, liveness, failure windows, and whether proposed repairs deliver the requested guarantees.

The task requires substantial work without an artificial minimum word count. Research cards must answer the actual question concisely; the critical memo can be short. The difficult part is the density of reasoning, not the length of the answer.

We compared three modes:

- **All Astra:** Astra handles every delivery unit.
- **HiRoute mixed:** the same work enters smart routing. Actual records show Qwen handling six research components and Astra handling the critical memo.
- **All Qwen:** Qwen handles the same delivery requirements through a fixed route.

Routing used generic criteria: explicit extraction, translation, verification, and formatting could use economy; novel guarantees, conflicting evidence, or complex failure interactions required primary. Product names and hidden reference answers were not used to assign models. Subject models received the complete frozen sources; the routing evaluation saw the current task, claims, and source metadata.

## Where the savings come from, and where quality matters

[![The two pairs meeting the same whole-delivery gate: mixed cost is below one tenth of all-Astra](assets/research-cost-en.svg)](assets/research-cost-en.svg)

The following pairs passed the original whole-delivery gate: all 360 cards delivered, at least 353 correct, and no material error anywhere in the critical memo.

| Pair | HiRoute mixed research cards | All-Astra research cards | Mixed / Astra critical memo | Conservative cost reduction |
| --- | --- | --- | --- | --- |
| Pair 2 | 357/360 | 359/360 | Pass / Pass | **92.01%** |
| Pair 3 | 356/360 | 359/360 | Pass / Pass | **91.39%** |

Costs include subject attempts and routing evaluations, using frozen API tariffs and exchange rates. The conservative reduction compares the mixed upper cost bound against the all-Astra lower bound. This table presents the two pairs that passed the original gate; all three pairs remain in the experiment record.

The critical recommendations are particularly revealing:

| Mode | Critical memos without material errors |
| --- | --- |
| HiRoute mixed | **3/3** |
| All Astra | **3/3** |
| All Qwen | **1/3** |

The two all-Qwen failures were not JSON formatting mistakes. Some repair suggestions treated a local transaction binding SQL state and offsets, or an outbox, as a solution for atomic visibility across SQL and an independent Kafka output. Those techniques have legitimate uses, but do not alone remove the cross-system visibility window required by the task.

That is the value of the allocation: economical models can perform much of the evidence work, while a small number of consequential judgments justify stronger reasoning. All-Qwen's research-card accuracy was itself close to the mixed mode; the meaningful gap appeared in the critical recommendations.

## Experiment two: one instruction, about 32 minutes, 343 passing assertions

To test autonomous handoff, we used a pinned HTTPX coding task: add synchronous and asynchronous streaming JSON iteration, correctly handle JSON, NDJSON, JSON text sequences, encodings, and stream-consumption state, and yield available values before requesting the rest of the body.

A native agent executed code and tools. The initial task required both implementation and verification; no operator guidance or product-code patches were supplied during execution.

[![Recorded long task: Qwen starts, Astra takes over after a context handoff, and all 343 independent assertions pass](assets/unattended-handoff-en.svg)](assets/unattended-handoff-en.svg)

| Observation | Recorded result |
| --- | --- |
| Initial task instructions | 1 |
| Intermediate operator prompts / manual code patches | **0 / 0** |
| Native execution time | **31 min 36 sec** |
| Actual model requests | 25 Qwen, 15 Astra |
| Upgrade after the second compaction | Competence **0.485**, below the **0.5** floor |
| Actual subsequent execution | 13 Astra tool calls |
| Independent acceptance | **108 feature + 229 regression + 6 incrementality assertions, all passing** |

Independent verification followed execution, with protected tests unchanged. This case demonstrates an actual automatic upgrade followed by accepted completion. It has no all-Astra cost control and does not support a 90% savings claim. [Task, parameters, and verification](../experiments/cases/unattended-engineering/README.md)

### Follow the execution record, not just two scores

A useful handoff connects three kinds of evidence: **why escalation happened, which model actually executed afterward, and whether the final delivery passed acceptance.** This historical record supports the following observations:

| Stage | Actual model and assessment | What it establishes |
| --- | --- | --- |
| Intermediate summary | Astra, stage score **0.775** | Assesses that stage only; it is not a score for the final code repair |
| Around the second context compaction | Qwen, stage score **0.485**, below **0.5** | The routing record shows the competence guard selecting primary |
| Subsequent implementation and repair | Astra, **13 actual tool calls**; no later stage score | Primary continued executing, rather than merely being selected |
| Independent checks after completion | **All 343 assertions passed** | The final implementation met this task's acceptance requirements |

These stages involved different work. **0.775 versus 0.485** is not a same-task score comparison between Astra and Qwen. Historical assessments also carry partial-evidence flags and must be read alongside their coverage. The stronger evidence in this case is the connection from escalation to actual execution to independent acceptance. [Inspect decisions, compaction timestamps, and model request records](../experiments/cases/unattended-engineering/results/2026-10-04/evidence.json)

## Start with a long task of your own

Choose a familiar task with a clear acceptance standard. Connect model sources, create a route in Desktop, connect your preferred agent, and inspect actual execution in the session record. Your own execution evidence can help identify which work suits economy and which needs stronger reasoning.

“Same quality” in this article means meeting the same predefined delivery standard, not identical answers or scores. The 90% figure concerns API-equivalent costs for two accepted research pairs, including subject calls and routing evaluation. It is not a subscription-invoice reduction or a promise for every task. The long-task case demonstrates autonomous handoff and completion, with no cost-savings claim.

Start with [HiRoute installation](https://hiroute.ai/en/download/), or recalculate the published results without model calls:

```sh
python3 experiments/reproduce.py verify
python3 experiments/reproduce.py report
```

The [experiments directory](../experiments/README.md) preserves all nine research deliveries, item-level findings, cost provenance, tasks, and rerun entry points. Offline replay makes no model calls; fresh execution uses your own connections and writes separate results. Semantic review still requires reading every answer against its sources; structure checks alone are not quality grades.

These are purpose-designed cases with three research repeats per mode and unblinded evaluation. Complete original records, supplementary analysis, and transport recovery remain available for inspection under the same definitions.
