# Article illustrations

The two explanatory slides have Chinese and English versions:

- [Long-task engine, Chinese](assets/long-horizon-engine-zh.png) / [English](assets/long-horizon-engine-en.png).
- [Competence and cost, Chinese](assets/competence-feedback-zh.png) / [English](assets/competence-feedback-en.png).

These are explanatory presentation graphics created with the built-in `image_gen`
tool, not Desktop screenshots or measured output. Their text describes product roles
and the routing policy discussed in the article. The terminology follows the project's
long-task positioning: “Spend less. Stay steady. Choose smarter.”

The [complete generation and correction prompts](illustration-prompts.json) record the
requested text and visual constraints. Chinese and English text and arrows were visually
checked.

The cost chart and recorded HTTPX handoff graphic are generated separately by
`experiments/render_figures.py` from the published evidence. Their measurements come
from the experiment records, not image generation.

The native Desktop screenshots ([Chinese](assets/desktop-session-quality-zh.png) /
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
