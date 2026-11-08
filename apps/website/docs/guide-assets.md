# Guide screenshot sources

The bilingual guides follow two independent journeys: model routing for model
selection and cost, and Agent routing for delegation across execution harnesses.
The UI calls the latter **Task routing**; keep that label in procedural steps.
Do not describe planned enterprise APIs or commercial offerings as available UI.

## Dark-mode captures

`public/guide-assets/*-{zh,en}.webp` were captured on 2026-11-08 from Desktop
production React components at source revision
`a3e3446b151ac6d8e5e99fd422858183b36ab55a`.
The website change does not alter those components.

| Asset | Production interaction | Synthetic host data |
| --- | --- | --- |
| `models-add` | Models → Add models | `interaction-audit-fixtures.mjs` |
| `trial-entry` | Home → New models | `new-model-trial.tsx`, Home view |
| `trial-setup` | Connect and try → Choose models | `trial-quickstart-fixture.tsx` |
| `agent-editor` | Agent → Codex → Model routing | `interaction-audit-fixtures.mjs` |
| `routing-worker` | Smart routing → More settings → Task delegation | `interaction-audit-fixtures.mjs` |
| `agent-task` | Agent → Codex → Task routing | `interaction-audit-fixtures.mjs` |
| `tasks` | Agent → Task history → a running task | `interaction-audit-fixtures.mjs` |

Fixtures are under `apps/desktop/tests/v3/browser/`. English captures translate
the sample route names at the host boundary. All controls, layouts and navigation
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

The guides also reuse the dark `readme-trial`, `readme-report` and `readme-session`
assets. See [their provenance](../../../decision-extensions/assets/README.md):
trust and report values are illustrative; the session is a real run of an example
task. Figure captions retain this distinction.

## Keeping the guides usable

Keep URLs stable, preserve the literal button labels, and show the next action
and a completion check for each journey. Keep detailed runtime rules in native
`details` elements after the core steps. Images link to their full-size source,
have descriptive alt text, and load lazily. Update Chinese and English together.
