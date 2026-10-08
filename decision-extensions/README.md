# Decision models and routing mechanism

[简体中文](README.zh-CN.md) · [Custom extension API](api/README.md) · [Official Jev extension](extensions/jev-decider/README.md)

HiRoute uses a decision model to choose the models for the current task and, when
enough evidence is available, assess the preceding execution stage. You configure
the task conditions and model groups; HiRoute applies them on each new user turn.

## Connect a decision model

1. Open **Models → Decision models** and add a connection. Built-in connections
   support **Bailian**, **OpenRouter Jev**, **TypeSafe**, and compatible System One
   endpoints.
2. Enter the provider, model, complete endpoint and credential, then save and test
   the connection.
3. Select the saved connection in a routing plan and configure smart saving or
   custom task branches with their execution models.
4. Publish the plan to apply it. Editing a connection or a draft takes effect only
   after the plan selects that version and is published again.

Built-in decision models need no self-hosted service. The connection test checks
the saved connection's transport and required response fields with synthetic input;
it does not establish the accuracy of decisions on your tasks.

A **custom extension** is an optional HTTP service that integrates its own provider
and follows the task definitions and assessment standards sent by HiRoute. Add it
from the same page using its complete endpoint. The form provides copyable request
examples and an OpenAPI download. The [official Jev extension](extensions/jev-decider/README.md)
is a deployable reference implementation.

## Choose a routing mode

| Mode | What the decision model judges | Execution models |
| --- | --- | --- |
| Smart saving | Whether the current task is simple or complex | Economy and primary groups |
| Custom branches | The current task category, then its required degree when it has two groups | Regular models and optional primary models per category |

Category, degree and competence answer different questions:

| Question | Example | Effect |
| --- | --- | --- |
| What is the current task? | Writing or reviewing an article | Select one task branch |
| What degree of work does this task require? | A local wording edit or a full argument review | Select a model group within that branch |
| How well did the preceding stage perform? | A draft has useful structure but unsupported claims | Record competence and, when applicable, protect the next turn with primary models |

For custom branches, overlapping conditions are resolved by the current request's
main intent; no match uses the configured default branch. A category with only
regular models skips degree evaluation and can still collect competence scores.
Smart saving uses one task scope, so simple and complex are not separate task branches.

## How HiRoute selects and executes models

For a task with two groups, regular/economy models are selected when this call's
simple probability meets the threshold (default **0.8**) and there is no applicable
complete score below the competence floor (default **0.5**). Otherwise HiRoute
selects primary models. Low-score protection uses only a fresh score for the
preceding actual stage with the same category, published plan version and assessment
standard. Missing assessments remain unscored; assessments based on partial evidence
retain that marker and are excluded from averages. Neither is treated as zero or
triggers low-score protection. A saved old score does not become a fresh low score
on later turns. A writing score does not upgrade a review task.

Each new user message is decided again and may choose either group. Tool
continuations and same-turn replay reuse the frozen decision only when HiRoute can
recognize the same turn and that decision remains reusable. Discontinuous
reconstructed history or a decision that cannot be inherited requires a new judgment.
The preference to keep the current model applies only among eligible models in the
newly selected group.

HiRoute tries the selected group's candidates in order. If regular/economy
candidates are exhausted, it can relay to the same category's primary group.
Primary exhaustion fails the request. Starting in primary keeps relay within that
group; a single-group category fails when its candidates are exhausted. A candidate
failure does not create a competence score.

If the category is valid but its degree result is unavailable, HiRoute uses that
category's primary group. If the whole decision fails, smart saving uses its
heuristic rules; custom branches use the default category and its primary group
when configured.

## Adjust judgment settings and inspect results

Advanced judgment settings start collapsed. Expand them to edit the simple/complex
criteria, thresholds, assessment instructions and the **0 / 0.5 / 1** competence
anchors. Category conditions describe task intent, degree conditions describe work
within a category, and competence standards assess work that has already happened.

Custom categories follow the plan's judgment settings by default. Independent
editing copies the complete settings; resetting resumes following the plan.
Smart saving also offers explicit heuristic rules, which do not produce model
competence scores.

Published definitions, connection versions and assessment standards are frozen.
Session and plan views show why a group was selected, the group and model that
actually executed, and any later score for that stage. Later scores and draft edits
do not rewrite earlier selection reasons.

## Built-in integration and custom extension responsibilities

| Responsibility | Owner |
| --- | --- |
| Task definitions, assessment standards, thresholds and model groups | HiRoute, from the published plan |
| Selection, eligibility, availability relay and actual-stage observations | HiRoute |
| Direct System One provider calls and answer mapping for built-in connections | HiRoute's [built-in adapter](api/system-one-design.md) |
| Provider calls, answer mapping and necessary history trimming for a custom connection | The custom extension, following the supplied definitions and standards |

The custom [extension API](api/README.md) can return the current decision and a
preceding-stage assessment in one call. Its current **v1** has one request shape;
the [canonical examples](api/decision-examples.json) and [OpenAPI](api/decision.openapi.json)
describe that boundary. Complete current input is preserved, while missing target
evidence must remain visible as partial. The [routing guide](../docs/smart-saving-model-classification.md)
describes the product configuration.

Tool subset selection is future protocol documentation only; it has no supported
runtime or product entry.
