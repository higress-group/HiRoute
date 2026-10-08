# Decision documentation illustrations

## Native session captures

The README, homepage and model-routing guide use
[English](quality-native-en.png) and [Chinese](quality-native-zh-CN.png) captures
of the real macOS Desktop Sessions page. Both are 2400×2100 pixels, captured at
1200×1050 logical points, with the navigation, session list and two complete
model-performance cards visible. The website scales them proportionally to its
content width without cropping or fixed-height distortion.

Capture used the production Desktop/daemon and isolated observation stores,
not browser IPC mocks. Computer Use produced the window images; JPEG-to-PNG
conversion preserved their dimensions without repainting or compositing the UI.

The stores contain constructed order-callback repair records: Qwen 3.8 Flash in
the economy group scores 0.37, and Qwen 3.8 Max in the primary group scores 0.91.
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
| Smart saving | [English](config-en.png) | [中文](config-zh-CN.png) | Decision method, economy/primary candidates, collapsed judgment settings |
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
