# Guide screenshot sources

The bilingual guides follow two independent journeys: model routing for model
selection and cost, and Agent routing for delegation across execution harnesses.
The UI calls the latter **Task routing**; keep that label in procedural steps.
Do not describe planned enterprise APIs or commercial offerings as available UI.

## Dark-mode captures

The original guides' captures below were made on 2026-11-08 from Desktop
production React components at source revision
`a3e3446b151ac6d8e5e99fd422858183b36ab55a`.
The website change does not alter those components.

| Asset | Production interaction | Synthetic host data |
| --- | --- | --- |
| `models-add` | Models → Add models | `interaction-audit-fixtures.mjs` |
| `trial-entry` | Home → New models | `new-model-trial.tsx`, Home view |
| `trial-setup` | Connect and try → Choose models | `trial-quickstart-fixture.tsx` |
| `agent-editor` | Agent → Codex → Model routing | `interaction-audit-fixtures.mjs` |
| `agent-management` | Agent → Codex → HiRoute management → Enable | `interaction-audit-fixtures.mjs` |
| `routing-worker` | Smart routing → More settings → Task delegation | `interaction-audit-fixtures.mjs` |
| `agent-task` | Agent → Codex → Task routing | `interaction-audit-fixtures.mjs` |
| `tasks` | Agent → Task history → a running task | `interaction-audit-fixtures.mjs` |

Fixtures are under `apps/desktop/tests/v3/browser/`. English captures translate
the sample route names at the host boundary. The management capture uses a known
not-configured Skill state and a missing CLI entry to show both setup actions.
All controls, layouts and navigation
are production components; no UI labels or screenshot pixels are edited. These
captures illustrate configuration and status, not a live provider run, benchmark,
price offer or savings claim. They use no personal credentials or task content.

Reproduce from the repository root, with Desktop dependencies installed:

```sh
npm --prefix apps/desktop ci --ignore-scripts
node apps/website/scripts/capture-guide-assets.mjs
```

The script starts its own Vite server and headless Chrome on loopback, uses a
disposable browser profile, and stops its processes on exit. Set `CHROME_BIN` if
Chrome is not at the platform default. It captures 1200 × 900 dark-mode WebP
images in both languages. Inspect every changed image before publishing.
Set `GUIDE_CAPTURE_STATE=agent-management` (or another asset name above) to
refresh only that interaction in both languages.

## Agent integration captures

`public/guide-assets/agent-guide-*-{zh,en}.webp` use the production Desktop
components at `82fb2fbc866231535494dc551e3c8df2a0e70d1e`, captured on
2026-11-10. The capture opens real product forms with example plan names and
disposable host replies. The Magpie endpoint, Qoder model IDs and displayed
model names are drawn from a local Magpie 0.1.3056 and HiRoute 0.6.0 setup.
Only those example configuration facts are carried into the screenshots.

| Asset | Where to look |
| --- | --- |
| `agent-guide-claude` | Advanced settings in Model routing, with Sonnet mapped |
| `agent-guide-qoder`, `agent-guide-pi`, `agent-guide-dsh`, `agent-guide-opencode` | Adding a published route to the respective Agent |
| `agent-guide-hermes` | The Hermes profile and its main model route |
| `agent-guide-magpie-api` | Custom API configured with the local URL and Responses |
| `agent-guide-qoder-models` | The two Qoder models in General models |
| `agent-guide-difficulty` | Heuristic rules and both model groups |
| `agent-guide-pi-worker`, `agent-guide-dsh-worker` | Selecting the executor under Task delegation |

The subscription and Desktop onboarding captures below use the production
components at `51403b63ea71d14420ccd3a55a10485debd5e33c`. Subscription catalogs use example model labels and
disposable host replies. The Desktop detail shows the route-selection and
separate-window launch controls.

| Asset | Where to look |
| --- | --- |
| `agent-guide-subscription-login` | Codex and Claude Code sign-in buttons |
| `agent-guide-codex-subscription-models`, `agent-guide-claude-subscription-models` | Selecting subscription models and saving the connection |
| `agent-guide-codex-desktop` | Selecting the only route for the separate Desktop |
| `agent-guide-codex-desktop-launch` | Opening the configured Desktop window from HiRoute |
| `agent-guide-workbuddy-pi` | Selecting the WorkBuddy route for Pi |

Capture the full group after installing Desktop dependencies as above:

```sh
GUIDE_CAPTURE_GROUP=agents node apps/website/scripts/capture-guide-assets.mjs
```

Use `GUIDE_CAPTURE_STATE` to refresh one pair. These images illustrate setup;
the shared session assets below show what a user can inspect after a real run.

The guides also reuse the dark `readme-trial`, `readme-report` and `readme-session`
assets. See [their provenance](../../../decision-extensions/assets/README.md):
trust and report values are illustrative; the session is a real run of an example
task. Figure captions retain this distinction.

### Native Codex Desktop windows

`agent-guide-codex-desktop-window-{zh,en}.webp` are user-supplied native Codex
Desktop screenshots from 2026-11-10. They show an empty project chat with the
`Qoder Responses` route selected. The supplied light appearance and full
2560 × 1640 window are preserved, with lossless WebP encoding. These external-app
images are shared by the Codex and Qoder tutorials and are not generated by the
HiRoute component capture script. They show the window and selected route, not
a completed model request. Refresh them from matching native-app captures when
that interface changes.

## Keeping the guides usable

Keep URLs stable, preserve the literal button labels, and show the next action
and a completion check for each journey. Keep detailed runtime rules in native
`details` elements after the core steps. Images link to their full-size source,
have descriptive alt text, and load lazily. Update Chinese and English together.

At release preparation, follow [documentation maintenance](../README.md#documentation-maintenance)
and the [release review](release-reviews/README.md). Recapture affected flows when
their visible controls, labels or behavior change; inspect both languages and
update the capture revision above (or record per-asset revisions when they differ).
Do not refresh every image mechanically for an unrelated change. A reused capture
needs a concrete `unchanged` assessment in the release review; that record binds
the actual image bytes and provenance to the product candidate being reviewed.
