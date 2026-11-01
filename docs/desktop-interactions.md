# HiRoute interaction guidelines

This is the shared interaction reference for macOS Desktop, Windows Desktop and
the authenticated Web console. Feature contracts and authoritative service state
still determine what an action does. Keep equivalent capabilities recognizable
across these hosts; explain differences through actual host capabilities.

## Start from the user's job

Before adding or changing an interaction, find its production owner in the
[code map](code-map/README.md) and compare an existing equivalent flow. Identify
the object, the intended effect, whether input is a draft or already saved, and
the state that makes the operation available. Reuse that interaction when those
meanings match. A different provider or implementation is not by itself a reason
for another enable, edit, save or recovery workflow.

Keep useful distinctions. Filtering a list, choosing a provider to open its form,
switching pages and selecting a single setting are different jobs. Do not make
them identical merely because they can all be rendered as buttons.

## Common patterns

| Intent | Interaction and wording | Owning production pattern |
| --- | --- | --- |
| Navigate between peer views | Labelled page tabs; selected state, keyboard access and clear content ownership. Preserve or explicitly guard drafts. Arrow keys focus guarded tabs; Enter/Space activates through the existing guard. | Shared `TabList`, `ProductPage` and the active feature's navigation |
| Filter or change a list view | Compact labelled controls beside the affected list; keep selection when it remains valid. | Models and Sessions list controls |
| Edit configuration | Label above each input; group related fields; place optional detail after the fields needed for the main job. | Connection, Agent and route editors |
| Cancel an edit | **取消 / Cancel** leaves without committing; guard actual unsaved changes and retain context when the user continues editing. | Shared discard guard |
| Return within a flow | **返回 / Back** returns to the preceding step and preserves its draft. Do not use it as an undisclosed discard action. | Connection editing session and trial flows |
| Close information | **关闭 / Close** dismisses a read-only view. Closing feedback does not cancel the underlying operation. | `Dialog` and operation feedback |
| Commit configuration | Name the effect: Save, Enable, Publish, or Save and test. One main action at the trailing edge; Cancel/Back precede secondary and primary actions in both visual and keyboard order. | Shared dialog/footer and editor actions |
| Test a connection | State the tested scope and any provider quota/cost before submission. Opening a page or ordinary Save must not silently add a model call. | Connection and decision test controls |
| Disable or restore | Keep the action beside the named capability and distinguish it from permanent removal. Preserve independent capabilities and the existing authoritative restore flow. | Agent capability sections and source controls |
| Remove or clear saved data | Separate the action from routine editing. Confirmation names the target and explains the actual scope and retained data; references and authoritative impact checks remain binding. | Model/decision removal and session-clear dialogs |
| Retry a failed read | **重试 / Retry** reads the affected state and retains input. | Feature error/recovery callouts |
| Resolve an uncertain write | Re-query the original operation. Do not substitute another Save, replay a mutation or claim success from a timeout. | Operation observer and feature recovery owners |

Use ordinary buttons for actions that open a draft or confirmation. Use a switch
only when its checked state describes the actual setting and its action follows
the setting's save model. Do not expose a confirmation-based Enable as an
immediate toggle. A provider directory entry remains a navigational action, not
a radio option pretending that configuration is already complete.

Discard confirmations keep **继续编辑 / Keep editing** as the focused safe choice
and separate the destructive discard action. An explicit Save in credential
management can reapply its current set; it does not imply a live connection test.
Keyboard activation keeps focus on the selected tab after its content changes,
including peer views rendered in separate page frames. Focus must not fall back
to the document and restart the user's navigation from the sidebar.

## Editing and operation state

- Distinguish unchanged, edited, submitting, confirmed, failed and unverified.
  Saved/Enabled labels come from accepted state, not from a click or a successful
  request dispatch. Unknown state is neutral; a genuine zero remains zero.
- Read-only or disconnected state disables writes and explains why. Cancel, Back,
  Close and reading existing information remain usable unless an actual in-flight
  commit makes leaving unsafe. Do not trap these actions inside a disabled fieldset.
- Prevent duplicate submission while an operation is in flight. Keep failed input
  editable and retain the original operation identity when its outcome is unknown.
  Changing a selection must not silently replace a confirmation's captured target.
- Use current authority and operation state when executing an action. Preserve
  appropriate cancellation, draft ownership and recovery as that state changes.
  Keep detailed timing regressions in the owning automated scenarios.
- Treat secrets according to their existing protected-input owner. Do not copy a
  Key to another provider, serialize it into UI state/evidence, or expose it in
  diagnostics. Preserving a draft does not authorize changing credential ownership.
- Keep the common save model clear within each panel. Immediate preferences and
  explicit configuration saves can coexist in separate areas with honest feedback;
  do not introduce a new global autosave or collective Save over independent effects.
- Limit synchronous work to what completes the current action. Navigation and
  typing should not trigger unrelated full scans, history aggregation or inference.

## Layout and visual hierarchy

Keep the HiRoute mark, provider identity and semantic palette. Use quiet, flat
surfaces, a small number of type/spacing/radius roles and one main action per
active work area. Use accent color for identity, selection and the main action;
status color represents actual state. Avoid decorative gradients, glows, repeated
tinted icon tiles and shadows on resting panels. Floating dialogs, menus and
popovers may use elevation.

Use the production styles and shared primitives. A field group owns its spacing:
do not add generic field margins to its grid gap. Controls in one context share
height and alignment. Main copy, field labels and supporting text have distinct
roles; supporting text must remain readable. Keep ids, hashes and implementation
facts out of the main reading order when a meaningful name/status is available.
Retain useful diagnostic codes and exact identities in appropriately scoped detail.

Use the available width and let long names, addresses and explanations wrap.
Small windows scroll the fields while actions remain reachable. A footer or toast
must not cover the last field, error or action. Do not use overflow clipping to
hide a layout failure, CSS order to contradict keyboard order, or arbitrary blank
space to balance unrelated panels.

Empty, loading, error, unavailable and saving states retain the same page frame.
Explain the affected object and a useful next step. Keep persistent failures
available until resolved; a transient success notice must not replace a required
recovery action. Do not add example traffic, prices or successful tests to make
an empty screen look active.

## Implementation and acceptance

1. For affected flows, record the entry, analogous existing pattern, state
   transitions, authority/side effects and any intentional difference. Keep
   detailed domain requirements in their owning feature contract.
2. Implement through shared components/tokens where the behavior is actually
   shared. Keep domain intent, drafts and operation ownership with the feature;
   avoid a generic form engine or parallel global operation store.
3. Exercise opening, editing, Cancel/Back, submission and the relevant error,
   pending, read-only and recovery states. Use real keyboard input for focus,
   tab order and disclosure/navigation acceptance where those behaviors change.
   Select cases by the changed behavior and reuse existing regression coverage;
   this is not a per-change manual checklist. A presentation-only adjustment does
   not require unrelated persistence, authorization or lifecycle acceptance.
4. Capture before/after for each changed interaction using the same data, state,
   locale, theme and viewport. Keep full original screenshots and their actual
   revisions. Explain the practical change and its tradeoff; use geometry or
   other observable measurements when they establish the improvement.
5. Use default and minimum supported windows, Chinese/English and affected
   light/dark states as selected by [UI quality](../.agents/skills/hiroute-ui-quality/SKILL.md).
   Reuse reliable unchanged-state evidence instead of repeating an unrelated full
   matrix. A shared primitive change also needs representative affected consumers.
6. A reproduced defect needs an assertion that rejects the old behavior: for
   example action reachability, footer clearance, retained input, keyboard focus
   or a non-mutating Cancel. Presence checks, CSS-string tests and screenshot
   counts alone do not establish completion.
7. Separate component/fixture results from actual native authorization,
   persistence, OS effects and provider calls. Bind each result to its source;
   explain an intervening change before reusing evidence.

Record intentional exceptions beside their owning pattern with the user need,
affected hosts/states and acceptance evidence. Routine choices within an approved
interaction need no additional workflow approval. This reference does not change
the task's authorization, human review/merge authority or native-effect boundaries.
