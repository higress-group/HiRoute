# Decision documentation illustrations

These documentation previews use HiRoute's real `HomeNavigation`, `RoutingPage`,
`PlanEditor` and `PlanQuality` components and shared product styles. They were
captured in Chromium on the configured macOS workbench, not from a native Desktop
window. Model names come from the repository catalog; scores, dates, sessions and
task outcomes are fabricated. Every image is labeled as illustrative, not a model
benchmark. No model evaluation or paid provider request was performed.

| Illustration | English | Simplified Chinese | Purpose |
| --- | --- | --- | --- |
| Connect a decision service | [config-en.png](config-en.png) | [config-zh-CN.png](config-zh-CN.png) | Selected mode, endpoint, timeout, authentication and explicit test action |
| Interpret competence | [quality-en.png](quality-en.png) | [quality-zh-CN.png](quality-zh-CN.png) | Stage scores, coverage, partial evidence and an unrated stage |
| Inspect a session stage | [session-en.png](session-en.png) | [session-zh-CN.png](session-zh-CN.png) | The session's compact model-performance component and assessment coverage |

The configuration illustration uses the official endpoint
`http://127.0.0.1:8080/v1/decisions`. Do not treat this illustration as a successful
connection test.

## Placement in the article

Use the matching-language configuration image next to the setup instructions.
Use the matching-language performance image when explaining stage assessments
and when introducing plan specialization. The four displayed samples belong to
different demonstration sessions, not four successive stages of one unchanged
execution configuration.

The example is an order-service maintenance plan (revision 3) with DeepSeek V4.1
Flash and Claude Sonnet 5. Separate sessions concern pagination validation,
reconciliation debugging, payment idempotency and callback retries. Scores include
0.88, 0.37 with partial history, 0.91 covering turns 6–9 while execution has reached
turn 10, and an unrated stage. The sidebar includes local-fix and research plans.
The simulated differences are not evidence of either model's real capability or
of specialization improving performance. Inspect multiple real samples and
execution evidence before an authorized agent proposes or changes a plan.

No textual assessment reason is invented: Jev currently omits `reason`.
Evidence buttons are disabled because this fixture
has no underlying transcript. Do not claim these screenshots demonstrate
evidence navigation, scoring persistence or an end-to-end decision call.

Additional protocol-dialog and evidence-view screenshots require synthetic
retained transcripts and their own capture. Keep those
images next to the corresponding instructions; do not illustrate controls that
have not been implemented. An Agent-turn boundary diagram belongs beside the
mechanism explanation because a screenshot cannot show that timing reliably.

## Reproduce

The development-only entry is
[`apps/desktop/decision-docs.html`](../../apps/desktop/decision-docs.html) and its
[`fixture`](../../apps/desktop/decision-docs.tsx), with
[`data`](../../apps/desktop/decision-docs-data.ts). Neither is referenced by the
production entry or emitted by the default build. The fixture supplies only the
read responses needed by the product components and rejects other IPC commands.

From `apps/desktop`, run:

```sh
npm run dev -- --host 127.0.0.1 --port 5186 --strictPort
```

Open `/decision-docs.html?lang=en&view=quality`, replacing `en` with `zh` for
Chinese and `quality` with `config` or `session` for the other crops. Use the configured
macOS workbench and a dedicated browser profile. If Vite runs on Linux, forward
the loopback port over SSH, along with the isolated Chrome CDP port. Then run:

```sh
node --experimental-strip-types --test tests/decision-docs-data.test.mjs
node tests/decision-docs-capture.mjs <local-cdp-port> http://127.0.0.1:<mac-vite-forward-port> --capture
```

The capture command checks all six views, score filters, model labels, and read-only
IPC; omit `--capture` to verify without writing images. It fixes the browser clock
to September 20, 2026 and the timezone to Asia/Shanghai for repeatability. Data in
an interactive preview uses the current time so the seven-day filter keeps working.
Use light theme, scale 1, device scale factor 2: 1600×880 for quality; width 1200
with heights 400 for session, 980 for English config and 1050 for Chinese config.
The quality view scrolls the real editor to its performance section. Only the
canvas and crop are styled; product controls, row layout and text are unchanged.

Capture provenance: product components and shared styles from the same repository
revision as these images. The documentation wrapper and images are updated together.
These checks do not validate native window integration, evidence navigation, model
scoring or persistence.
