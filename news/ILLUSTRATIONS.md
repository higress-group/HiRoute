# Article illustrations

The two explanatory slides have Chinese and English versions:

- [Long-task engine, Chinese](assets/long-horizon-engine-zh.png) / [English](assets/long-horizon-engine-en.png).
- [Competence and cost, Chinese](assets/competence-feedback-zh.png) / [English](assets/competence-feedback-en.png).

These are explanatory presentation graphics created with the built-in `image_gen`
tool, not Desktop screenshots or measured output. Their text describes product roles
and the routing policy discussed in the article. The overview is the article's only
slogan-bearing figure. The body explains the
product in terms of connection, routing, and observation rather than repeating it.

The [complete generation and correction prompts](illustration-prompts.json) record the
requested text and visual constraints. Chinese and English text and arrows were visually
checked.

The cost chart and recorded HTTPX handoff graphic are generated separately by
`experiments/render_figures.py` from the published evidence. Their measurements come
from the experiment records, not image generation.

The earlier native Desktop screenshots ([Chinese](assets/desktop-session-quality-zh.png) /
[English](assets/desktop-session-quality-en.png)) were captured on 2026-10-04 from
an isolated macOS app displaying an exact subset of the long-task observation
metadata: one session, 40 requests, four quality segments, their receipts and
safe execution facts. This is historical metadata replay, not a new model run
or a screenshot captured during the original experiment. No account credentials,
conversation bodies or agent connection configuration were imported.

The app was resized and its built-in zoom set to 150% for readability. PNG pixels
were not retouched or generated. The native capture tool reported restricted
screen access; the returned app images were visually checked against the app's
accessibility snapshots. Existing partial-assessment and missing-content indicators
were retained. The raw score is 0.485; the app formats it to two decimal places.
These screenshots illustrate observation, not configuration or independent acceptance.
They have been removed from the article body pending a corrected native capture: the
model labels expose internal identifiers, and the evidence badges do not explain their
coverage. The original session also has content-capture gaps; omitted replay content
is not a complete explanation for its incomplete-content status. The assets remain
for provenance, not as a claim that those presentation defects have been fixed.

## Native capture requirements for the revised article

Capture the real native app after the display fixes, in both languages. Record the
app revision, capture date, route revision, source of the displayed data, and whether
this is historical replay or a new run. Do not label a reconstructed configuration
as a screenshot taken during the original experiment. Do not retouch labels, scores,
evidence states, or charts. Use the app's own layout and zoom for readable framing.

| Capture | Required visible content | Placement and purpose |
| --- | --- | --- |
| Research mixed plan | Smart saving mode; Qwen-3.8 Flash in Economy group at xhigh; GPT-6 Astra in Primary group at medium; readable connection model name and active state | After the setup steps: show what the two model groups look like in the product |
| Long-task mixed plan | The two actual model groups and their reasoning settings; readable plan identity and active revision | With the parameter table if it materially differs from the research-plan view; explain policy parameters in the caption rather than inventing UI controls |
| Session handoff | Readable Qwen/Astra names, stage boundaries, score coverage, 0.485 record (or its clearly identified UI rounding), and actual subsequent Astra requests/tools | With the execution-evidence section: connect the guard decision to work that followed |
| Evidence detail | A real relevant request or execution record and the precise evidence-coverage explanation, without credentials or unrelated personal information | Optional close-up where it makes the reason for escalation easier to verify |

The final Astra repair stage has no recorded later competence score. Keep that state;
do not replace it with 0.775 from the earlier summary stage. A screenshot can display
both models' stages but cannot establish a same-task score comparison. The independent
343-assertion result is separate evidence and should be presented as such.

Use configuration screenshots to document what is configured. Use the historical
experiment files to establish what was executed. If a new run is needed to obtain
complete evidence, identify its results separately instead of silently substituting
its scores into the frozen case.

## Overview revision brief

The next overview should explain one concrete model-call path: the user's existing
agent sends a request using a stable route name; local HiRoute selects the Qwen or
Astra group; the response returns to the agent for continued tool execution. Place
stage assessment below the route, with a return arrow at context handoffs. Keep
optional multi-agent delegation outside this core diagram. Show product roles and
actions instead of repeating three slogan cards. Proposed prompts are recorded as
pending below the original generation history; no replacement image has yet been
produced from them.
