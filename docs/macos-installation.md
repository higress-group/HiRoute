# macOS installation candidate

[简体中文](macos-installation.zh-CN.md)

The target is macOS 15+, with separate single-architecture packages for Apple Silicon
(`arm64`) and Intel (`x86_64`). The minimum version comes from the actual build facts of the
bundled official CPA release. Older macOS, real Intel hardware, and Windows require separate
evidence and cannot be inferred from an arm64 run. The repository provides candidate-build
entry points; a successful build is not Finder, account, Worker, or public-distribution
acceptance.

## Installation and CLI

Open the candidate DMG, drag `HiRoute.app` to its `Applications` shortcut, eject the image,
and open `/Applications/HiRoute.app`. A ZIP remains available for direct extraction.
A self-signed package may be blocked on first launch because it has no Developer ID
notarization. An engineering candidate is not automatically suitable for public distribution;
only the exact artifact that passes integrity, installation, component-disclosure, and release
review and appears in the website manifest is a public download. Use a trusted package and
follow the macOS application-open confirmation flow without disabling system security.

The app can install its per-user terminal entry only while running from
`/Applications/HiRoute.app` or `~/Applications/HiRoute.app`. In Settings → CLI, explicitly
choose Install. HiRoute creates `~/.local/bin/hiroute` as a symlink to the bundled CLI; it
does not copy the binary, request `sudo`, or modify shell files. If that path already contains
a non-HiRoute entry, installation reports a conflict and leaves it unchanged. The entry is
not installed while the app runs from a DMG, download directory, or another location.

Ensure that `~/.local/bin` is on `PATH`; zsh users can add this to `~/.zprofile`:

```sh
export PATH="$HOME/.local/bin:$PATH"
hiroute worker list --output json
hiroute --help
```

Settings reports the symlink as missing/valid/broken/conflict, whether `PATH` includes its
directory, whether an earlier command shadows it, and whether the daemon is available. A
valid symlink does not prove the service is running. Repair/remove operates only on a HiRoute-
owned link targeting a supported `HiRoute.app`; it never deletes `~/.local` or
`~/.local/bin`. A managed Agent prepends `~/.local/bin` to its subprocess `PATH` only after
revalidating the link against the current trusted app CLI.

Managed Agent skills and launchers continue to use the `hiroute` command and existing
integration Preview/confirmation flow. Running the app requires no Rust, Go, Docker, npm, or
source checkout. A Worker's own CLI, ACP adapter, and required runtime follow the dependency
guidance in Settings and are not bundled with HiRoute.

The CLI connects by default to
`~/Library/Application Support/ai.hiroute.desktop/run`. `HIROUTE_RUNTIME_DIR` takes
precedence over `XDG_RUNTIME_DIR`; both are explicit development/test overrides. An empty or
relative path is unavailable and never falls back to another instance. Open HiRoute when it
is not running. After a service outage, restore Desktop and query with the original operation
or task identity rather than resubmitting a request with an unknown outcome.

CPA is bundled and used only for authorized subscription paths. If it is missing, replaced,
or not executable, subscription checks fail while control and other available API sources
remain usable. Replace a damaged app with a complete candidate; do not install another CPA
manually.

## Manual upgrade and removal

1. Explicitly quit HiRoute and confirm that this service instance and its tasks have stopped.
   Closing the window only hides the application. Full lifecycle acceptance is tracked
   separately; a vanished window alone is not stop evidence.
2. Replace `/Applications/HiRoute.app` while preserving application data, then launch it. A
   fixed location keeps the per-user symlink target stable.
3. If the app moved or integration checks report an invalid path, use the integration Preview
   and confirmation in Settings. Resolve user-configuration drift instead of overwriting it.
   Check and, if needed, disable/re-enable the login item.

Before removal, Preview and restore managed model/Agent integration from Settings, disable
background launch at login, explicitly quit, and delete the app. App deletion preserves data
by default. For a complete data removal, first confirm that no task is running, integration
has been restored, and sessions/logs/receipts are no longer needed; then delete only the
current user's HiRoute data under
`~/Library/Application Support/ai.hiroute.desktop`. Never delete `.codex`, `.claude`, user
projects, or existing credentials. If the app is damaged, restore a complete candidate to the
same installation location before restoring integration; do not guess at configuration
overrides by hand.

## Build and checks

Run from the exact clean committed checkout on macOS (the host architecture by default):

```sh
RUSTC_WRAPPER=sccache python3 scripts/package-desktop.py build \
  --arch arm64 --cpa-source-repo /path/to/CLIProxyAPI
# Use the actual paths reported by the build:
python3 scripts/package-desktop.py verify /absolute/output/HiRoute.app
python3 scripts/package-desktop.py verify-dmg /absolute/output/HiRoute-VERSION-SHA-macos-arm64-trial.dmg
```

The build reuses the existing shared Rust-validation lock and keeps output in this checkout's
`target/`. Even with sccache, never move the Cargo target. Packaging performs a locked release
build and frontend build; ordinary development validation remains Debug.

From the explicitly supplied CPA checkout, the tool resolves the commit pinned by
`vendor/cpa/source.json`, verifies and applies the adjacent launch-pipe patch, builds with Go,
signs CPA, and compiles the final digest manifest into Desktop. It never performs deep
resigning. The patch only lets the parent pass existing instance-management credentials over
an inherited pipe; loopback and remote-management-disabled rules remain intact and the model
protocol is unchanged. The build host needs Go and access to locked Go dependencies. An
arbitrary prebuilt CPA is rejected and nothing is downloaded at user startup.

The upstream license is installed as `Licenses/CLIProxyAPI-LICENSE`; source, patch, compiler,
and artifact digests appear in build evidence. The packager also generates
`THIRD-PARTY-LICENSES.txt` and a machine-verifiable manifest from locked Cargo/npm dependencies
and the same CPA commit, placing them with HiRoute's Apache-2.0 license under
`Contents/Resources/Licenses`. Missing verifiable license material aborts the build.

Intel uses `--arch x86_64` and requires the `x86_64-apple-darwin` Rust target. Apple Silicon
uses `--arch arm64` and `aarch64-apple-darwin`. A cross-build must be able to execute the target
CPA version probe (Apple Silicon therefore needs an existing Rosetta installation for Intel);
the tool never installs Rosetta. Architectures have independent output under
`target/desktop-package/CANDIDATE_SHA/ARCH/ROUND/`; components are not mixed and no universal
binary is synthesized.

Rerun the same command for another round. UTC time and a random suffix keep rounds separate
without overwriting artifacts or cleaning `target/`. DMG/ZIP names contain version, twelve
commit characters, architecture, and either `trial` or `developer-id`. Each `result.json`
contains the full commit, DMG/ZIP SHA-256, component digests, and image checks; `build.log`
retains command output. The DMG is a compressed read-only image built with `hdiutil`, then
checked for integrity, a read-only mount entry, and app components before being detached. If
detach fails, packaging fails and logs the mount path. Close windows using that path and run
`hdiutil detach PATH`; never forcibly detach another image.

For manual review, first compare `shasum -a 256 FILE.dmg` with the round's `result.json`, then
install as described above. Quit the old app before replacement. Use a separate macOS test
account for an initial isolated check when possible, and do not present success under a
development override as ordinary Finder-launch acceptance. Record the complete commit (also
in `Contents/Resources/installation.json`), DMG SHA-256, architecture, macOS version,
reproduction steps, and actual result. If macOS blocks launch, record the message and approve
the trusted app through System Settings → Privacy & Security. Do not remove quarantine
attributes or disable Gatekeeper/SIP. An ad-hoc build may still require manual approval, and a
successful build does not guarantee launch.

The native host, daemon, CLI, and CPA are in `Contents/MacOS`; the current model catalog is
compiled into the daemon and frontend assets into the host.
`Contents/Resources/installation.json` records candidate commit, version, final digests, and
system dynamic dependencies. `dependency-inventory.json` is an attribution-review inventory,
including Cargo dependencies that may not be linked, and does not replace license texts.

A Developer ID candidate uses an existing notarization keychain profile while collecting
component licenses through the same locked process:

```sh
python3 scripts/package-desktop.py build \
  --cpa-source-repo /path/to/CLIProxyAPI \
  --identity 'Developer ID Application: YOUR ORGANIZATION (TEAMID)' \
  --notary-profile YOUR_PROFILE
```

Credentials come only from the local keychain and never enter the repository or command
output. The tool signs nested binaries and the app, submits and staples notarization, rebuilds
the ZIP, then packages, signs, and notarizes the DMG; both notarizations must be Accepted.
Self-signed builds are marked `controlled-trial` and make no Developer ID or notarization
claim. That technical marker alone does not establish component disclosure, installation, or
release review. Near-term public self-signed releases still require explicit applicable
component material and all release-manifest gates.

Record installation evidence separately for installation location/Finder launch without
overwrite; CLI access to the same service and not-running errors; real delegation/results;
authorized subscription calls; health/control behavior when CPA is missing, replaced, or
fails; login launch; quit; manual upgrade; and recoverable removal. Include the exact commit,
package digest, and actual result. Component and fixture tests cannot replace this evidence.
