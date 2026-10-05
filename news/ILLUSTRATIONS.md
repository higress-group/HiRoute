# Article illustrations

The article's explanatory slides have Chinese and English versions:

- [Product overview, Chinese](assets/long-horizon-engine-zh.png) / [English](assets/long-horizon-engine-en.png).
- [Competence and cost, Chinese](assets/competence-feedback-zh.png) / [English](assets/competence-feedback-en.png).
- [Local architecture, Chinese](assets/local-architecture-zh.png) / [English](assets/local-architecture-en.png).

These are explanatory presentation graphics created with the built-in `image_gen`
tool, not Desktop screenshots or measured output. Their text describes product roles
and the routing policy discussed in the article. The body introduces the product,
then presents two case studies, the local architecture and the download entry.
Configuration pictures accompany the first case; competence, handoff and observation
pictures accompany the second.

The [original generation and correction prompts](illustration-prompts.json) and
[architecture prompts](local-architecture-illustrations.json) record the requested text
and visual constraints. Chinese and English text and arrows were visually checked.
The product overview appears once in the introduction to explain the positioning
and the slogan. The local architecture figure near the end explains runtime
responsibilities and the computer boundary. Both are retained because they serve
different purposes.

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

## Configuration screenshots

The article's three setup steps use six native macOS captures made on 2026-10-04:

| Capture | Chinese | English |
| --- | --- | --- |
| Model management | [Models](assets/desktop-models-zh.png) | [Models](assets/desktop-models-en.png) |
| Smart Saving plan | [Routing](assets/desktop-routing-zh.png) | [Routing](assets/desktop-routing-en.png) |
| Codex model-routing setup | [Client access](assets/desktop-route-access-zh.png) | [Client access](assets/desktop-route-access-en.png) |

The app source revision is `89925c86bc8b4993759a8ce137c2e905f17d5e72`.
The isolated Desktop and daemon used the managed Debug Pilot build at that revision.
The final captures use the app's **100%** text setting, as requested, and a native
1920 × 1050 window. The capture output is 1920 × 1080 including the top margin.
PNG pixels were not cropped, retouched or generated. The capture tool reported
restricted screen access; every returned image was visually checked against the
real app and accessibility snapshot.

This is a configuration demonstration reconstructed through the normal Desktop UI,
not a capture taken during the original experiment. The route `Astra × Qwen · Smart
Routing`, alias `hiroute-smart`, was published and reopened at revision 2: Qwen-3.8
Flash in Economy at `xhigh`, GPT-6 Astra in Primary at `medium`. The configuration
uses the built-in classifier to illustrate the model groups; it does not stand in
for the experiments' recorded decision policy or request traces.

The source named `Research models` used a loopback, catalog-only endpoint and a
non-production key. Model names and configurable capabilities were entered through
the source editor; the demo uses a 131,072-token context limit and 32,768-token output
limit. These settings are demonstration declarations, not measurements of upstream
capabilities. The endpoint served only the two model IDs and rejected inference.
No upstream model calls, subscription refreshes or account credential imports were
performed. No price was assigned, so the source page correctly shows it as unpriced.

The Codex picture shows the ordinary setup dialog with the published route selected.
The final client-enable action was not applied to the operator's Codex configuration.
It demonstrates the setup entry, not a new end-to-end model test. The original model
calls, cost figures and acceptance results remain in the frozen experiment files.

## Observation evidence

The final Astra repair stage has no recorded later competence score. Keep that state;
do not replace it with 0.775 from the earlier summary stage. A screenshot can display
both models' stages but cannot establish a same-task score comparison. The independent
343-assertion result is separate evidence and should be presented as such.

The article uses the recorded handoff figure and the linked execution notes to explain
the second experiment. It does not reuse the earlier flawed observation screenshots
as current product evidence or substitute a new score into the frozen case.

## Competence product views

The four images are native captures of HiRoute Desktop on macOS:

- Session model performance ([Chinese](assets/desktop-session-competence-zh.png) /
  [English](assets/desktop-session-competence-en.png)): actual execution models,
  reasoning settings, stage scores and evidence actions.
- Routing-plan model performance ([Chinese](assets/desktop-plan-competence-zh.png) /
  [English](assets/desktop-plan-competence-en.png)): model configurations grouped
  by execution branch, scoped averages, and scored/unrated stage counts.

Desktop reads an isolated copy of the unchanged observation records retained from
this HTTPX case. The images come from the production UI and local daemon, without
mock IPC, generated interface text or image edits. Both languages were selected
through the app's settings, with 100% text size in all four captures. The Chinese
routing list translates plan names and purposes in the isolated replay catalog;
the English list retains the source names. Plan IDs, revisions, model candidates,
execution identities and scores are unchanged. The main competence views display
`gpt-6-astra`; the full CPA transport ID remains in expandable stage details.
Reasoning settings use the original tier names (`medium` and `xhigh`) in both
languages.
Native validation covered stage and assessment-trigger
navigation, exact model drill-down, unrated filtering, scope changes, refresh on
re-entry, and preserving draft settings without changing published-plan statistics.
Capture hashes, binary hashes and the source evidence archive are recorded in
[the capture provenance](competence-product-views.json).

Qwen has one scored stage at 0.485; Astra has one at 0.775 and one unrated stage.
The existing two-decimal display produces 0.48 and 0.78. The earlier 0.410 assessment
lacks an attributed visible execution stage and is excluded from this product-view
average. A prior 0.615 assessment of the later Qwen stage is not another independent
stage. No score has been added to the final Astra stage.

Stage scores describe different stretches of work, not a controlled model ranking
or whole-delivery acceptance. Retained diagnostic states and raw identities are
available in expanded stage details. The captures validate the product's observation
interface; they do not constitute a new model-quality experiment. Each article uses
matching localized images and the same original numerical values and stage counts.

## Local architecture

The architecture figures show one concrete call path: an existing agent calls the
local HiRoute Gateway, which connects to cloud or locally hosted model services.
Desktop/CLI configures the gateway, and execution facts feed local observation.
The computer boundary includes the agent, management, gateway, records and local
models; cloud services remain outside it. Connected model services perform inference.

The Rust core, shared Desktop/CLI service and local record storage are described by
the public source references in the architecture prompt record. The figures explain
those responsibilities rather than prescribing internal process or crate boundaries.
The article describes the Higress engineering lineage separately from HiRoute's Rust
implementation. The existing product overview remains in the introduction; the
historical prompt file also retains an earlier, unexecuted redraw brief.
