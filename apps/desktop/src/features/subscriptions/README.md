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
its native results separate from a selected independent-login candidate.

`SubscriptionApiConnection.tsx` owns optional existing-API selection and inline Key
entry for the registered OpenAI API (Codex) or Anthropic API (Claude). The subscription
flow keeps one primary save action and no nested API dialog or model picker.
`ModelManagementPage.tsx` validates input, confirms the subscription save, and only
then calls `new-api.ts` to check/save exact API-observed selectable counterparts of
the saved subscription membership. The mounted inline credential owner survives
loading and completion states. API failure retains the subscription and offers
API-only retry or finish-later; it never reuses a consumed subscription approval.
Saved-source association edits reuse the same inline setup.

`new-api.ts` owns the protected input and new API creation before association.
`save-attempt.ts` observes the original sealed request/key after uncertain writes.
A created API is retained and reused if association fails; pending association
metadata alone cannot prove its original Operation completed. Closing before
admission releases owned input, while a dispatched save retains native recovery
ownership even when replay itself is refused. After confirmed association, a
failed completion read retries only canonical readback; the association's own
model-sync revision is not an external-edit conflict. API transport policy is
independent of subscription proxy settings.
`SubscriptionApiCredits.tsx` links provider claim instructions and Key setup,
identifies eligible plans and excluded coding-client uses, and distinguishes API
verification from credit activation. Official billing owns eligibility and balance.
Ordinary API connection forms retain their separate service and model selection.

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
