# Managed CPA patch

`source.json` pins the upstream revision, artifact version and patch digest.
`local-management-stdin.patch` includes the parent-pipe management credential
bootstrap and Codex account model discovery. Build with `scripts/build-cpa.py`.

Codex OAuth registration queries `https://chatgpt.com/backend-api/codex/models`
using the account's access token and account ID over CPA's existing proxy/TLS
transport. `client_version` comes from the managed auth metadata
`hiroute_client_version`, populated by HiRoute's bounded native engine
`--version` probe. No fixed discovery version or local models-cache reader is
used. A new access lease re-probes the selected engine. Prerelease suffixes are
removed, matching Codex's models-manager.

Remote membership replaces the bundled membership for access-token accounts.
Existing bundled metadata can enrich matching IDs. Registration still applies
normal exclusions, OAuth aliases and account prefixes. Discovery has a five
second deadline and a 16 MiB response bound; redirects are rejected. Failure
leaves the existing registered catalog unchanged, without logging tokens or
upstream response bodies. An initial failure does not create model entries.
This does not add polling or a new catalog cache. Inference does not discover
models. Successful listing is not proof that inference succeeds.

HiRoute's ordinary routing compilation and Codex preview read and verify the
registered catalog without repeating an already-correct auth control PATCH.
An explicit subscription check refreshes it; credential/version changes use the
existing auth update path. This keeps remote directory availability out of
unchanged preview/publication reads while retaining all account pin checks.

Focused upstream tests are shipped in the patch:

```sh
go test ./sdk/cliproxy ./internal/runtime/executor ./cmd/server ./internal/api/handlers/management \
  -run 'TestCodexDiscovery|TestCodexModelDiscovery|TestRegisterModelsForAuth|TestLocalManagementPasswordPipeBounds|TestLocalMachinePasswordPreservesRemotePolicy'
```

Managed children allowlist HTTP(S)_PROXY and NO_PROXY (both cases). The local
patch applies Go's standard per-destination environment proxy selection to
protected-host TLS transports when no explicit proxy/injected transport exists.
It preserves NO_PROXY and does not retry direct after proxy failure. ALL_PROXY
is not part of this contract; configure HTTPS_PROXY for subscription endpoints.
