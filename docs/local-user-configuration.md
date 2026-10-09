# Local configuration saves

HiRoute's owner-only Local Control socket admits processes running as the same OS user as
`hirouted`. Desktop and CLI use this one local-user scope for configuration saves and Operation
recovery. They do not need a Desktop/CLI-specific Apply capability or a second authorization
confirmation. A Save, Publish, or Use this installation click is the ordinary user intent.

The daemon still validates the exact normalized change, digest, resource revisions, idempotency
key, and durable writer admission. On a conflict, refresh the current state and submit the
intended edit again. If a response is lost, first query or replay the same operation identity;
do not invent a new key or assume that a missing response means failure.

This local trust decision does not authorize remote Agent collaboration, Gateway calls, or
potentially billable model checks. Those flows keep their own consent and credential boundaries.
The Desktop WebView can only use its explicitly exposed native commands and cannot supply an
arbitrary Local Control request.

The routing editor displays the current head's call state separately from unsaved
edits and saved drafts. A disabled route remains visibly stopped even while its
configuration is being edited; only a successful lifecycle change enables calls.

## Codex CLI profile

For an isolated instance, set `HOME` and `CODEX_HOME` to its private directories.
`HIROUTE_CODEX_AUTH_SOURCE` may independently select an absolute, owner-only Codex
`auth.json` for subscription discovery and CPA's access-only lease. Set it on the
process that starts the daemon. Without this override, the source remains
`CODEX_HOME/auth.json` (or `HOME/.codex/auth.json`). An explicit invalid or missing
source never falls back to another account. Discovery and CPA use the same source;
the override does not select the native Agent configuration or Worker history root.
CPA never receives the source's refresh token and never refreshes or rewrites the
source. The original Codex retains refresh ownership.

An isolated acceptance process must also clear inherited provider credentials,
model-routing variables and Claude context overrides. A private `HOME` alone does
not remove higher-precedence process settings. A settings preview must explain
missing effective-configuration or safe-write evidence without exposing settings
values; it must not bypass those checks to make a save succeed.

The default Codex connection owns one `hiroute.config.toml` profile in the detected original
`CODEX_HOME`. It preserves `config.toml` and the ordinary CLI/Desktop default provider.
Copy the launch command from the Agent page and select your shell. The command explicitly
sets the detected `CODEX_HOME` for that invocation because a newly opened terminal may have
another environment; it does not change `HOME` or your shell configuration.

One canonical `CODEX_HOME` supports one managed model connection: either this CLI profile
or the advanced default-configuration takeover. The advanced mode changes the default provider
for ordinary Codex and Desktop using that home. To switch, restore the current model settings,
finish any pending cleanup, and then configure the other mode. An existing unowned profile
or an inherited `model_providers.hiroute`/`profiles.hiroute` entry is a conflict; HiRoute does
not adopt files by name or remove their authentication fields. Preserve your own files and
resolve the conflicting entry manually before a fresh preview.

Profile mode uses native `codex --profile hiroute`; it does not provide a Desktop profile
launcher. History can differ between entry points. Cross-provider history restoration and
session database changes are outside this feature. Gateway credentials authorize the selected
models independently of the Codex client's OpenAI login; retaining original models still
requires a usable upstream source configured in HiRoute.

A pending operation reserves the home even if Gateway access is already revoked. The Agent
page shows the target and conflicting fields, and can recheck and resume the original Operation.
Save your edits before repairing the file. A file change after the operation was staged,
including formatting, must be undone before retry; after cleanup you can reapply unrelated
edits. HiRoute never force-overwrites a conflict. A newly created profile containing user-added
settings is retained after the managed fields are removed.

Same-user CLI callers can resume that same operation through `agents connect apply` with:

```json
{"schema":"hiroute.agent-settings-retry/v1","context_id":"<discovered context>","operation_id":"<pending Operation>"}
```

This retry accepts no new settings or grants. The daemon checks the stored operation and its
context before continuing. First-time creation and edits still require normal Preview/Apply.
