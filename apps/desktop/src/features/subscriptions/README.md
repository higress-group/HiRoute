# Subscription connections

`SubscriptionSignIn.tsx` owns the independent Codex/Claude sign-in interaction. The shared
`ManageSubscriptionLogin` contract is implemented through Client Core and the daemon; the
WebView never receives credential files, tokens or provider error bodies. Browser URLs live
only in the current interaction. Manual callback text is cleared on submission and enters
the native `submit_subscription_login_callback` command, which registers the exact pending
session's protected input reference before ordinary Local Control submission.
The pinned CPA receives normal browser callbacks on temporary loopback listeners.
The existing optional `reason_code` identifies manual fallback when that listener
is unavailable: show guidance before an explicit browser-open action. A received
callback alone never establishes login; the existing status observer must confirm
authorization. The manual completion form remains available in both modes.

List/status recover server-owned sessions after reopening. Closing an interaction cancels
the pending sessions it started, including a start response that arrives after closing.
An authorized session remains a candidate: `ModelManagementPage.tsx` opens the existing
check → model selection → Preview/Apply save flow. Selected subscription details use
the subscription title and hide device-discovery controls, including Hermes profile
scanning; returning to the scan list restores those controls. No sign-in action
publishes a route.

The native reuse option opens the existing device scan. Background token synchronization
does not grant refresh authority: native clients retain their refresh tokens. Copy must
distinguish this dependency from an independent sign-in's automatic credential refresh.
The device scan explains that dependency before connection and links to the existing
independent sign-in interaction. Opening that entry releases the prior scan check;
it neither starts authorization nor saves a connection. Product copy calls this
automatic sign-in credential refresh, never renewal of the paid subscription.

Device scans explicitly request native-only candidates. Independent HiRoute sign-ins
stay in login management, while login continuation retains the complete candidate
query and saved-source append uses exact source/revision lookup. The scan list keeps
its native results separate from a selected independent-login candidate. The associated API form initially selects OpenAI for Codex or Anthropic for Claude.
It snapshots the subscription's explicit model selection and, after an API check,
saves only exact observed/selectable counterparts without opening the API model
picker. Empty selections require choosing subscription models first; a check with
no matching models retains the form and subscription selection. Ordinary API model
selection and explicit service changes keep their existing behavior.

Tests belong to the Desktop subscription suites and the production Pilot path; shared
contract and protected callback admission regressions belong to Application API, CLI and
the daemon. Generated CLI schemas and Tauri permissions are finalized by the convergence
owner with the same candidate as the backend.

Independent sign-in is the recommended entry. The saved-source view joins explicit V3
subscription-mode records by source ID and revision; unavailable credentials never change
a saved mode. Reauthorization opens independent sign-in for a CPA-managed source and
native discovery for a borrowed source. Changing mode still requires Check and Save.
The published CLI management queries retain their strict V2 responses.

Codex borrowing uses file credentials only. A selected non-file credential store refuses
stale auth.json and directs the user to independent sign-in; Claude keeps its existing
file and explicit Keychain check. Error copy separates missing login, unsupported mode or
store, unreadable credentials, missing account identity and runtime/network unavailability.

Current association saves use V5, independently of Native network V4. Retained
association V4 receipts keep their original wire bytes and protected source proofs.
The exact association-only schema35 layout is upgraded through the normal backed-up
three-store pipeline to schema36, adding main's Native policy companion without
recreating association rows or rewriting journals. Ambiguous/corrupt layouts fail
closed. The compatibility register owns retirement of this retained input support.
