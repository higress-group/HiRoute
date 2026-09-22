# Use smart model routing

Smart model routing decides which model an agent should use for the current stage of work. HiRoute decides only at natural boundaries where reselection is allowed. Ordinary tool continuations remain on the same model so a stage can keep reusing its prefix cache.

The sections below explain the product through Desktop. Linux headless uses the same routing contract through `routing options/list/show/preview/apply`. Start with [Linux headless installation](/en/docs/install-linux/) and [HiRoute CLI](/en/docs/cli/), and treat the installed schema as authoritative.

## Choose a routing mode

Create or edit a plan under Smart routing, then choose How to use models:

| Mode | When to use it | Behavior |
| --- | --- | --- |
| Fixed model | You need a predictable candidate order | Tries candidates in order and skips ones that lack required capabilities or are temporarily unavailable |
| Smart saving | You want economy or primary models selected by task need | Sends simple work to the economy group and complex work directly to the primary group |
| Free first | You want free models attempted first | Tries free candidates in a fixed order, then either stops or uses an optional primary fallback |

Candidates must satisfy the request's text, image, tool, and context requirements. Once response delivery begins, HiRoute does not silently switch models within that response.

## Configure Smart saving

1. Add models suited to routine, bounded work to the Economy group.
2. Add models for complex work to the Primary group.
3. Decide whether an unavailable economy group may continue to primary. Complex work never downgrades to economy.
4. Publish with the built-in rules first. Add an external decision service only when you want Jev, an LLM, or your own policy.

![Smart saving configuration; the image uses an illustrative setup](/decision-assets/config-en.png)

An external service is optional. When enabled, it implements the general [Decision API](/en/docs/decision-api/) to choose among the plan's allowed branches and may assess the prior execution stage. The official [Jev decider](/en/docs/jev-decider/) is a self-hosted reference implementation.

The service receives accepted assistant text plus ordered tool names and coarse outcomes, never tool arguments or results. Messages maps explicit `is_error`; Responses maps explicit function-output and provider web-search states. Chat tool results, custom outputs, or a missing explicit status remain `unknown`: HiRoute does not infer failure from free-form result text.

## Understand decision boundaries

New user input creates a decision opportunity. During autonomous work, compaction and the context rebuild that follows provide another natural opportunity to choose again and assess the previous stage. HiRoute's routing engine determines whether the existing decision can still be inherited; the client does not need to detect or report compaction separately. Consecutive tool continuations stay within the current stage.

A new decision does not force a model change. HiRoute can keep the existing branch when it still fits. Different models cannot share a KV cache, but deciding at a boundary where context already needs to be rebuilt avoids breaking a stable prefix merely to classify an ordinary tool continuation.

## Publish and connect an agent

Select Enable to publish the route. Then open Agents, choose the target agent, open Model routing, turn on Use HiRoute model routing, select the plan, and choose the default.

Saving confirms that the configuration was applied. Use the page's live check or a real task to validate the model path; do not treat a successful save as proof of upstream inference.

## Inspect the outcome

- Use Sessions to see the actual plan, branch, and model for a request.
- Use Runtime performance on the plan to inspect stage competence over time for models selected by the active revision.
- Unrated is not zero, and partial history is not a complete conclusion. Read a score with its plan revision, covered range, and evidence.

Competence blocks risky cost-cutting; complexity creates opportunities to save. HiRoute never rewrites or splits a plan automatically because of one low score.
