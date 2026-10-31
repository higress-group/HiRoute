# Decision documentation illustrations

## Native session captures

The homepage and model-routing guide use
[English](quality-native-en.png) and [Chinese](quality-native-zh-CN.png) captures
of the real macOS Desktop Sessions page. Both are 2400×2100 pixels, captured at
1200×1050 logical points, with the navigation, session list and two complete
model-performance cards visible. The website scales them proportionally to its
content width without cropping or fixed-height distortion.

Capture used the production Desktop/daemon and isolated observation stores,
not browser IPC mocks. Computer Use produced the window images; JPEG-to-PNG
conversion preserved their dimensions without repainting or compositing the UI.

The stores contain constructed order-callback repair records: Qwen 3.8 Flash in
the regular group scores 0.37, and Qwen 3.8 Max in the primary group scores 0.91.
There are two scored stages, a 0.5 competence floor and a 0.8 simple-probability
threshold. The low first-stage score activates primary protection on the next
stage. Three request records retain execution and assessment context without
creating a third model stage. These are documentation fixtures, not provider
benchmark results or evidence of live ingestion. Public captions describe the
product capability; capture provenance is maintained here.

No credentials or
existing observation stores were copied. The original development app and system
language preference were restored after capture. A native rendering check does
not replace routing or provider acceptance.

## README session recordings

The root README uses [English](readme-session-en.png) and [Chinese](readme-session-zh-CN.png)
frames from macOS screen recordings of real DeepSeek Harness Desktop sessions routed
through HiRoute Desktop release builds, not browser mocks. Both sessions work on the same
fictional Account Export project with live provider calls: DeepSeek V4.1 Flash in the
regular group, GPT-6 Astra as primary, a 0.90 simple-probability threshold and a 0.50
competence floor. Scores, token counts and API-equivalent cost estimates are what those
sessions recorded; they are not benchmark results.

Each frame is cropped to the Sessions pane without repainting or compositing. The English
frame is 950×1046 from a 1× recording of one continuous six-turn session in which every
turn was fully observed. A low score on the third turn applied low-score protection to
the user's next message, and later turns returned to Flash. The frame comes from reviewing
that finished session with the protected turn selected, so the pointer rests on that turn.
Its build also contained two then-unmerged changes: a stage-assessment prompt revision
([#293](https://github.com/higress-group/HiRoute/pull/293)) and a DeepSeek Harness
adapter fix ([#295](https://github.com/higress-group/HiRoute/pull/295)). The Chinese frame
is 1900×2072 from a 2× recording, with the pointer resting on the selected turn.

## README trial and report captures

The README's [trust level](readme-trial-en.png) ([中文](readme-trial-zh-CN.png)) and
[model performance report](readme-report-en.png) ([中文](readme-report-zh-CN.png)) images
render the production trust-level control and model-performance report from the Plan editor in
headless Chrome at device scale 2, dark theme (matching the session recordings), 1200-pixel viewport
width. The trust image is cropped
to the slider and presets with Trusted selected; the report image to the report section.
Neither is repainted.

The data is synthetic: the browser fixtures under `apps/desktop/tests/v3/browser`, served
through the development-only [readme-docs.html](../../apps/desktop/readme-docs.html). That
entry gives the plan an English name for English captures and replaces the report's cost comparison with an illustrative
$3.42 routed versus $11.86 primary-reference estimate. Counts, suggestions and scores are
fixture values, not measured results.

To reproduce, run `node_modules/.bin/vite` from `apps/desktop` and open
`/readme-docs.html?polish=1&native=1&view=editor&theme=dark&lang=en`: click Trusted in the trust
control, then open Model performance. Use `lang=zh` for Chinese.

## Current product component captures

The current images use HiRoute's actual `HomeNavigation`, `DecisionServicesPage`,
`RoutingPage`, `PlanEditor` and `PlanQuality` components and shared product styles.
They are captured in isolated Chromium on macOS. They are not native Desktop
screenshots. Only the surrounding documentation canvas is styled; forms, tables,
selectors, provider logos and disclosure controls come from the product.

All data is synthetic. No real credentials, daemon,
model provider, saved user settings or paid calls are involved. Connection tests
are not simulated as passing. These captures demonstrate the current UI, not
provider quality, persistence, evidence navigation or native window acceptance.

| View | English | Chinese | What it shows |
| --- | --- | --- | --- |
| Decision models | [English](decision-models-en.png) | [中文](decision-models-zh-CN.png) | Two-pane model page, provider logos, Bailian connection form |
| Difficulty routing | [English](config-en.png) | [中文](config-zh-CN.png) | Decision method, regular/primary candidates, collapsed judgment settings |
| Custom branches | [English](custom-branches-en.png) | [中文](custom-branches-zh-CN.png) | Writing/review conditions, regular/primary groups and branch settings |
| Plan performance | [English](quality-en.png) | [中文](quality-zh-CN.png) | One row per configured execution slot; stage counts and averages |
| Session detail excerpt | [English](session-en.png) | [中文](session-zh-CN.png) | Actual stage selection, frozen threshold and later assessment coverage |
| Compact session excerpt | [English](quality-session-en.png) | [中文](quality-session-zh-CN.png) | Two stages in the session's model-performance component; not the homepage image |

The sample plans are order-service maintenance and writing/review (revision 3).
The writing branch uses Qwen 3.8 Max with distinct reasoning profiles; review uses
Qwen 3.8 Flash and Qwen 3.8 Max. Five stages span four sessions. Illustrative scores
include a complete 0.88, a partial 0.37 excluded from averages, an unrated stage,
and a review session with 0.38 followed by 0.91. The latter shows primary protection
from a fresh low score even though simple probability meets 0.8. It is not evidence
that one model outperforms another. Synthetic editor/publication IDs intentionally
differ so a configured slot and its observation must join into one row.

No textual Jev reason or transcript is fabricated. Evidence buttons are disabled
because the fixture has no retained conversation. The session images are component
excerpts, not the entire Sessions page or a demonstration of user-feedback navigation.

## Reproduce

The development-only entry is [decision-docs.html](../../apps/desktop/decision-docs.html),
with its [wrapper](../../apps/desktop/decision-docs.tsx) and
[data](../../apps/desktop/decision-docs-data.ts). None is emitted by the production
build. Mock IPC permits only the required read operations and rejects writes/tests.

From `apps/desktop`, start Vite:

```sh
npm run dev -- --host 127.0.0.1 --port 5186 --strictPort
```

Open `/decision-docs.html?lang=en&view=quality`. Replace `en` with `zh` and `quality`
with any view name in the capture script. Use a dedicated browser profile under
HOME, not a temporary directory or your everyday browser profile. Start isolated
Chrome with a loopback CDP port, then run:

```sh
node --experimental-strip-types --test tests/decision-docs-data.test.mjs
node tests/decision-docs-capture.mjs <cdp-port> http://127.0.0.1:5186 --capture
```

The script checks 12 bilingual views, exact model-row counts, stage filtering,
read-only calls and visible errors. It uses October 8, 2026 at 17:40 Asia/Shanghai,
light theme, scale 1 and device scale 1. Viewport sizes and scroll positions live
in the capture script. The compact excerpt is 1200×813. This script never writes
the `quality-native-*` images. Compare repeated captures for stable layout and content; browser edge anti-aliasing
can vary PNG bytes. Protocol artifacts generated by `decision-contracts.py` must
remain byte-identical on a second run. A documentation capture does not replace real product acceptance.

## Mechanism diagrams

The bilingual `jev-decision-*.svg` files illustrate the built-in decision path and
optional custom extension: category, degree and preceding-stage assessment are
separate questions, while HiRoute owns thresholds and execution. They are editable
vector diagrams, not measured traces.

The bilingual `readme-routing-comparison-*.svg` charts plot the reviewed-subset results of the
[decision-interface comparison](../../experiments/system-one-routing-paper/supplements/decision-interface-transfer/README.md).
