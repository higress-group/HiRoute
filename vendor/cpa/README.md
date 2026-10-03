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

## Current macOS development artifacts

The checked-in `development-cpa-artifacts.v1.json` identifies a historical
`.1` binary; its hash is retained as measured, not relabeled as the current
`.2` build. Do not stage that old binary with the current daemon.

On macOS, `python3 scripts/package-desktop.py build --cpa-source-repo /absolute/CPA-checkout`
generates the current patched CPA and a measured `cpa-artifacts.json` in its
candidate output directory. For a Debug Desktop build, set
`HIROUTE_CPA_MANIFEST=/absolute/candidate-output/cpa-artifacts.json` and use that
same output's `HiRoute.app/Contents/MacOS/cliproxyapi` beside the Debug `hirouted`:

```sh
python3 scripts/stage-desktop-cpa.py \
  --manifest /absolute/candidate-output/cpa-artifacts.json \
  --cpa /absolute/candidate-output/HiRoute.app/Contents/MacOS/cliproxyapi \
  --hirouted /absolute/checkout/target/debug/hirouted
```

Use the default ad-hoc package identity for this development staging path.
The staging tool rejects stale versions before copying; it still verifies the
exact digest, architecture, signature, dependencies and version of the binary.
A Linux backend test does not validate this native macOS build/install path.
