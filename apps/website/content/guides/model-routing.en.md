# Use smart model routing

Smart model routing selects and executes models according to the published configuration. Fixed model and Free first follow candidate order. Smart saving and Custom branches judge the task to select a model group and judge again for every new user message. For these two routing types, tool continuations reuse the frozen decision when HiRoute can identify the same turn, history is continuous, and the choice remains reusable.

This guide shows Desktop. Linux headless exposes the same production paths through `routing options/list/show/preview/apply` and `decision services list/apply/test`. Start with [Run HiRoute headless on Linux](/en/docs/install-linux/) and [HiRoute CLI](/en/docs/cli/); obtain request fields from the installed schema. For a first connection, follow [Quickstart](/en/docs/) to create a fixed route, then add decision models when needed.

## Choose a routing type

Create or edit a plan in Smart routing, then choose How to use models:

| Type | When to use it | Behavior |
| --- | --- | --- |
| Fixed model | You need a predictable candidate order | Tries candidates in order, skipping models that lack required capabilities or are temporarily unavailable |
| Smart saving | You want economy models for simple tasks and primary models for complex work | Chooses economy or primary using this turn's degree result and any applicable previous-stage score |
| Custom branches | Tasks such as writing and reviewing need different models | Selects a task category, then regular or primary models within that branch |
| Free first | You want free models attempted first | Tries free candidates in a fixed order, then either stops or uses an optional primary fallback |

Models → General models manages models that execute your tasks. Models → Decision models manages connections that judge and assess work, including self-hosted custom extensions. Decision models do not answer the task in place of execution models. Tool selection is not supported in this release.

## Connect a decision model

1. Open Models → Decision models and select Add decision model.
2. Choose a provider: Bailian, OpenRouter Jev, TypeSafe, or another compatible connection.
3. Enter an API key and connection name, then confirm the model name and complete decision endpoint. Expand prefilled values to edit them, or fill in any missing values.
4. Select Save and test, or Save only and later Test connection in the saved details.

HiRoute supplies judgment and assessment logic for built-in decision models, so no separate Jev deployment is required.

![Connect a decision model](/decision-assets/decision-models-en.png)

Save and test saves first, then tests that exact saved connection revision. The test sends fixed synthetic input and may consume provider quota. It reads no real conversation, executes no task, and produces no quality score. Passing confirms connectivity and required response fields for that revision in this test; it does not establish task judgment accuracy or continuing health. A failed test may still follow a successful save; check the status in the details.

Every saved edit creates a new revision. A route pins its selected connection revision: saving r2 leaves a published route on r1 until you select r2 in the route and publish changes. Saving or testing a connection does not publish a route. Adding a connection from an unfinished route preserves the draft; publishing remains a separate action when you return.

To implement your own judgment and assessment, choose Connect custom extension from the add menu, enter its complete endpoint, and select authentication. Extensions implement the [Custom extension API](/en/docs/decision-api/). See [Connect decision models](/en/docs/decision-extensions/) for built-in provider configuration and interface mapping. The optional [self-hosted Jev reference extension](/en/docs/jev-decider/) provides deployment examples.

During real routing, the selected decision connection receives the complete current user content and allowed execution history. History includes user content, accepted assistant text, ordered tool names, and explicit coarse outcomes. The request body excludes system/developer prompts, tool arguments, tool-result bodies, and execution-model credentials. Assessment uses criteria frozen when the target stage executed. Incomplete visible history and an incomplete assessment target retain separate markers.

## Configure Smart saving

1. Select Smart saving. Decision method defaults to Decision model; select the saved connection revision. You may instead choose Heuristic rules or Custom extension.
2. Add models for explicit, bounded work to Economy models.
3. Add models for complex tasks or protection after low competence to Primary models.
4. Use Add model to add candidates to each group, then adjust their order. Open a model's reasoning settings when needed and apply or cancel the change.

![Smart saving configuration](/decision-assets/config-en.png)

The main form needs only the decision method and two model groups. Judgment settings starts collapsed. The simple probability threshold defaults to **0.8** and the competence floor to **0.5**. Expand it to edit the simple/complex instructions separately from How competence is assessed, including assessment instructions and the 0 / 0.5 / 1 criteria. A higher simple threshold uses economy models more cautiously. An applicable complete previous-stage score below the competence floor selects primary for this turn.

Simple probability describes this turn's task degree, not the probability that an execution model will answer correctly.

Heuristic rules use task signals to classify simple or complex work and produce no model competence score. Probability thresholds and assessment prompts are hidden in that mode. Switching methods and returning preserves the selected connection and judgment draft.

## Configure Custom branches

Select Custom branches to assign different models to tasks such as writing and reviewing. Two branches are provided initially; add more when needed:

1. Give each branch a name and Task condition, such as “Draft or rewrite an article from supplied materials” or “Review an existing article and suggest changes.” Conditions describe task categories; put simple and complex criteria in Judgment settings.
2. Add Regular models. For two groups, choose Add primary models (optional) and add candidates. A branch with regular models only skips degree judgment but can still record competence for observation.
3. Expand Plan default judgment at the bottom to set shared thresholds, degree prompts, and assessment criteria.
4. Branches follow plan defaults. To use different criteria, expand the branch's Judgment settings and choose Customize. Choose Restore plan defaults to follow the plan again.

![Custom branch configuration](/decision-assets/custom-branches-en.png)

Customize copies the complete effective settings and makes the whole set independent, including thresholds, degree prompts, and competence criteria. Restore plan defaults removes the complete override and immediately follows the plan again, preserving the branch name, task condition, and models. A branch without primary models shows competence settings only; degree settings appear after primary models are added.

Each decision selects one task branch. Overlapping conditions follow the current main intent. If none matches, use the default branch under Follow-up preference and failure handling, then apply that branch's degree result. A low writing score remains attached to the writing stage; it cannot trigger primary review models merely because this turn asks for a review.

## Understand turn selection and failures

In Smart saving and Custom branches, a task with two model groups selects regular or economy only when this turn's simple probability meets its threshold and no applicable low score is present. Otherwise, primary is selected. With defaults, simple probability **0.8** meets the threshold and competence **0.5** meets the floor; a score below **0.5** triggers protection.

Protection requires a fresh, complete, valid assessment returned by this decision, bound to the preceding actual execution stage and compatible with the current category, published configuration, and assessment criteria. Missing, partial, cross-category, or unattributable scores cannot trigger protection. Unrated is not zero, and an old low score is not repeatedly applied.

A new user turn chooses again, so an earlier upgrade does not lock subsequent turns to primary. The next turn still uses its degree result and applicable score; it does not reset to economy unconditionally. Prefer the current model applies only within the group selected for this turn. Tool continuations and replay inherit the choice when HiRoute can identify the same turn, history is continuous, and the frozen decision remains reusable. If compaction or history reconstruction breaks continuity or prevents reuse, HiRoute judges again.

Candidate order provides failure relay. Exhausted regular or economy candidates continue to the primary group in the same task branch; exhausted primary candidates fail explicitly. A direct primary selection tries only primary candidates, without downgrading to regular or crossing task branches. A branch with one group fails when that group is exhausted. A failed candidate call is not a competence judgment and produces no zero score.

If a two-group branch's category is valid but its degree result is unavailable, use that branch's primary group and record a degree judgment failure. If the whole decision fails, Smart saving uses heuristic rules; Custom branches uses the default branch's primary group, or regular when it has one group only.

All candidates must satisfy the request's text, image, tool, and context requirements. Once response delivery begins, HiRoute does not silently switch models within that response.

## Publish and connect an agent

Select Enable or Publish changes to freeze the connection revision, task conditions, judgment settings, model order, and reasoning configuration. Missing models or connections and invalid judgment settings block publication. Invalid fields in collapsed sections are expanded and located.

Then open Agents, choose the target agent, open Model routing, enable HiRoute routing, select the plan, and choose a default. After saving, use the page's live check or submit a small real task to verify the model path.

Maximum wait per request includes retries and streaming output and defaults to 1 hour for new plans. A slow response is interrupted at this deadline. Request writing, waiting for the first response byte, and stream idleness each default to 10 minutes. Edit the total wait in the route; existing plans retain their saved value. This does not change the model's own maximum output length.

## Inspect the outcome

Open Model performance on the route to inspect stage samples and competence coverage by actual task branch, model group, model, and reasoning settings. Smart saving groups Economy / Primary. Custom branches groups positions such as Writing → Regular / Primary. The same model used in different groups is recorded separately.

Model performance in Sessions exposes the same stage facts, including the actual model, competence, and assessment coverage for each stage.

![Session stage assessments](/decision-assets/quality-native-en.png)

Open View assessment evidence or Stage details, then locate the session evidence and check these facts separately:

- **Turn selection:** task category, simple probability, the threshold at that time, selected group, and reason. Degree is not applicable when no judgment was needed and unavailable when it failed; neither means zero probability.
- **Actual execution:** the model and reasoning configuration actually used, candidate position, and any failure relay.
- **Later assessment:** a subsequent decision's score for this completed stage, its saved criteria, coverage, and completeness. Applicable low-score protection points to the old stage it assessed.

New drafts cannot rewrite past selection or thresholds. Read missing and partial scores with their coverage; HiRoute never rewrites a plan automatically because of one low score.
