# Subscription connections

`SubscriptionSignIn.tsx` owns the independent Codex/Claude sign-in interaction. The shared
`ManageSubscriptionLogin` contract is implemented through Client Core and the daemon; the
WebView never receives credential files, tokens or provider error bodies. Browser URLs live
only in the current interaction. Manual callback text is cleared on submission and enters
the native `submit_subscription_login_callback` command, which registers the exact pending
session's protected input reference before ordinary Local Control submission.

List/status recover server-owned sessions after reopening. Closing an interaction cancels
the pending sessions it started, including a start response that arrives after closing.
An authorized session remains a candidate: `ModelManagementPage.tsx` opens the existing
check → model selection → Preview/Apply save flow. No sign-in action publishes a route.

The native reuse option opens the existing device scan. Background token synchronization
does not grant refresh authority: native clients retain their refresh tokens. Copy must
distinguish this dependency from an independent sign-in's automatic renewal.

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
