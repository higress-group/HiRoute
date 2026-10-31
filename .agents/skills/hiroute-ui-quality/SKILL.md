---
name: hiroute-ui-quality
description: Design and verify consistent HiRoute Desktop/Web interactions, form and confirmation states, layout and visual hierarchy using production components and observable evidence. Apply to UI implementation or interaction/layout repairs; not backend-only work or discussion without implementation.
---

# HiRoute interaction and UI quality

Implement the confirmed interaction as a readable, consistent, usable real page.
Wrapping, hierarchy and state layout are delivery quality; compilation or control
presence is insufficient. Routine layout choices within the user's design need
no extra approval.

Use the [shared interaction guidelines](../../../docs/desktop-interactions.md)
when a UI flow is added or changed. Compare an existing equivalent capability
before choosing new controls, wording, save behavior or confirmation layout.
Read the relevant feature contract for actual authority and side effects; visual
uniformity must not erase meaningful domain or host differences.

## Find the production owner

Trace the live entry/component/style imports first. Desktop `src/main.tsx` loads
`src/occami/styles.css` and its `src/occami/styles/` imports; do not place fixes in
obsolete `src/style.css` based on its name. Test pages load production styles,
not a parallel style set hiding defects. Reuse actual Dialog/form/button components
and semantic color/spacing/radius/text variables. Check common layout assumptions,
e.g. horizontal `.field-label`; override vertically only in the affected form.
Fix dimensions only when the product requires them; controls/columns must shrink.

## Select coverage by the change

Record the affected entry, user job, analogous pattern and relevant state
transitions. Include Cancel/Back, busy, read-only, error and uncertain-result
behavior when affected. Check that non-mutating exits remain usable and that a
confirmation keeps its captured object/revision. No new approval gate is added.

Read the [Desktop test map](../../../apps/desktop/tests/README.md) and use
[HiRoute validation](../hiroute-validation/SKILL.md) to select and execute checks.
Record selected combinations:

- Layout/forms: default and smallest supported window from current `tauri.conf.json`
  (currently 1200×813 and 755×600). Distinguish content viewport from native window.
- Text/control widths: Chinese and English, relevant long labels/values/errors.
  Desktop currently fixes text scale at 100%; do not simulate unsupported settings.
  Preference changes check that old 150%/200% saves normalize to 100%. Add scale
  levels only when the product supports them again.
- Interaction: affected expanded/collapsed, loading, empty, error and saving states.
  Reliable unchanged-state evidence need not be mechanically repeated.
- Themes: affected light/dark. Existing semantic variables plus layout-only change
  normally need geometry in one theme and contrast sampling in the other.

## Inspect real pages and protect regressions

Capture a source-bound before/after for every changed interaction, with the same
data, state, language, theme and viewport. Keep complete original screenshots;
state the practical improvement and its tradeoff. For a shared primitive, cover
representative consumers and smoke the remaining affected entries. Comparison
is part of delivery quality, not a request to redesign already suitable flows.

Run production browser scenarios first. Native save/restart/authorization/real
requests require a capable Codex host and [HiRoute CU](../hiroute-cu/SKILL.md).
Claude does not load that Codex-only skill; report native acceptance pending.
Mocked IPC is not a business result. Save and actually inspect selected screenshots:

- Labels do not become vertical letters under control pressure; long text wraps
  naturally, primary/helper hierarchy is clear and field/card/button gaps agree.
- Controls align, long values stay within cards/dialogs, and hidden overflow does
  not conceal truncated content.
- Small windows scroll while main actions stay visible/clickable; footer cannot
  cover the last field/error. Expand/cancel/retry preserves context.
- Visible keyboard focus moves appropriately into/out of editing; disabled/saving
  states do not expose duplicate submission.

For a reproduced layout defect, add the smallest observable assertion under
production styles: label/control bounds, overflow or footer viewport. Show old
code fails and the repair passes. CSS-string matching/test counts do not prove
coverage. Geometry does not replace visual screenshot inspection.

Record revision, entry, viewport, language, text scale and state in the Issue/PR,
with selected screenshots/results and browser/native limits. Keep raw evidence
local unless explicitly suitable for publication. Documentation-only follow-ups
do not rerun unaffected product checks. Personal commands/credentials/host config
are not user examples. This skill grants no delegation, writes, merge or publishing
beyond the current task and repository rules.
