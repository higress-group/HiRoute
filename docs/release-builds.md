# Release candidate builds

The candidate entry builds macOS Desktop DMGs (`arm64`, `x86_64`) and Linux or Windows
headless archives (`x86_64`, `aarch64`) and a separate x64 Windows Desktop NSIS
installer. Windows headless archives are internal installer/test payloads only;
Windows publication consists solely of the x64 NSIS installer. A target's package integrity is separate from its required native runtime
acceptance and does not by itself authorize publishing that target. All components of one release must come from the same clean,
committed source revision, with CPA built from `vendor/cpa/source.json` and its
checked patch.

Omarchy Desktop uses a native Arch build from the same source revision with the
already qualified standalone archive and manifest for its architecture. Sign
the exact package and verify native installation with its detached signature
using the [Omarchy guide](linux-desktop.md). Publish the Arch package, detached signature and
SHA-256 file together. The publication workflow verifies the signature with
the public key at the released revision and compares the embedded archive and
manifest with the qualified inputs.

The candidate builder must reject a host/target mismatch, dirty source, external
Cargo targets, inconsistent component versions and wrong-architecture binaries.
It must retain a separate output directory for every attempt, use checkout-local
`target/` and the existing shared build lock, and preserve logs on failure.
The existing website release manifest remains the sole publication catalog;
candidate metadata is input for review, never an automatically published release.

## Product version lifecycle

All shipped product versions, their Cargo/npm lockfile entries and the generated
Local Control handshake schema/digest follow the Desktop Tauri version. Use the
shared tool rather than updating individual files:

```sh
python3 scripts/product-version.py check
python3 scripts/product-version.py advance
python3 scripts/product-version.py set 0.3.2
```

After recording stable `0.3.1`, the development line uses `0.3.2-dev.1`:
`0.3.1 < 0.3.2-dev.1 < 0.3.2`. An already planned higher minor/major or prerelease
is retained. `set` prepares an explicitly chosen development or release candidate
version; `advance` opens the next patch development cycle only when needed.
The independent `product-version` CI checks every PR/main push for consistent
components and a version later than the latest recorded stable release. Adding
only Git build metadata to the released number does not satisfy this rule.

Release numbers may skip intermediate patches or minors. To release `0.5.0`
directly from `0.3.2-dev.1`, run `python3 scripts/product-version.py set 0.5.0`,
commit the resulting changes, and build/tag that exact committed candidate as
`v0.5.0` through the normal publication flow below. A tag alone does not change
the compiled version. After stable `0.5.0` is recorded, advancement prepares
`0.5.1-dev.1`, preserving an already newer planned version.

The post-publication contract PR also prepares the next development version and
lockfiles. It requires normal review/merge, preserves an already newer planned
version, and never changes the frozen released source or snapshots. The binary
version, component handshake and package identity use this committed version;
the source SHA separately identifies the build. A development version does not
establish a stable release, protocol compatibility or feed publication authority.

## Build locally

Use the intended native platform with the repository Rust toolchain, Node 22+
(Desktop and packaging regression checks), Go matching the pinned CPA source,
sccache and native build tools. Linux additionally requires Docker: the same
command enters the pinned glibc 2.31 build image and then a separate minimal
runtime image. Host distribution libraries are never used as the release ABI
baseline. The native architecture check applies to the host and both containers.
APT resolves versions and verifies signatures using the pinned Debian snapshot
indexes. After that update, official Debian mirrors may supply package files
whose hashes match those indexes. For the pinned ARM64 snapshot, the
`libarchive13` package is also available by object ID from Debian Snapshot.
APT checks it against the SHA-256 in the signed index before using it.
Linux frontend preparation uses Node from the pinned build image, not the host.
The policy pins its version and each native architecture's official archive
SHA-256 from [Node's published checksums](https://nodejs.org/dist/v22.19.0/SHASUMS256.txt).
The image verifies the download before extraction and checks Node/npm execution;
candidate evidence records both versions. Mounting an arbitrary host Node would
couple the build to that binary's host libc requirements, outside the fixed
glibc 2.31 environment. Node is a build tool and is not added to the release payload.
The CPA checkout must contain the commit in `vendor/cpa/source.json`; its current
branch does not select the source. Invoke from the clean candidate checkout:

```sh
python3 scripts/build-release.py \
  --target x86_64-apple-darwin --cpa-source-repo /path/to/CLIProxyAPI

python3 scripts/build-release.py \
  --target aarch64-unknown-linux-gnu --cpa-source-repo /path/to/CLIProxyAPI

python scripts/build-release.py --windows-desktop \
  --target x86_64-pc-windows-msvc --cpa-source-repo C:/sources/CLIProxyAPI
```

The same entry accepts `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`. Windows builds require the
matching native MSVC host, Python, Go and sccache; PE machine identity is checked for
CLI, daemon and CPA. Its builder uses a checkout-local native file lock; it does not
import the Unix workbench lock implementation. The CPA probe retains the actual native
SystemRoot and uses isolated profile/temp paths. Windows candidates remain
`awaiting_runtime` until the installed foreground/background and feature gates pass;
see [Windows operating and evidence boundaries](standalone-windows.md). Internal
Windows builds write `internal-payloads/` and `publication: internal-only`, without
an `assets` directory or `website_artifact` row. They cannot be added to a release
catalog: both native and website validators reject Windows standalone entries
and Windows ARM64 packages.
The native x64 Windows Release invocation selects static CRT for Rust and its
CMake dependencies and rejects ambient compiler overrides. The complete archive's
three EXEs are checked for both ordinary and delay imports of Debug CRT or VC
runtime before candidate assets are accepted. Build evidence records the toolchain
digest, compiler context and exact image hashes. This dependency gate does not
replace installed runtime acceptance; Debug development builds and native ARM64
retain their separate build contexts and require their own evidence.
macOS candidates keep the existing macOS 15 minimum and self-signed trial
distribution. Developer ID signing and cross-building on a Mac remain available
through [package-desktop.py](../scripts/package-desktop.py) as documented in the
[macOS installation guide](macos-installation.md); the matrix entry uses native
hosts so its probes execute on the selected architecture.

`--windows-desktop` requires native x64 Windows and adds the shell to the same
static-CRT/image checks. It stages the verified standalone archive, including
console assets, under the shell's ignored resources, builds the pinned Tauri shell,
and bundles NSIS in `currentUser` mode without code signing. The output name is
`HiRoute-VERSION-REV12-windows-x86_64-setup.exe`. This stays separate from the
headless archive jobs. 7-Zip (`7z` or `7zz`) is required to extract and audit the
actual embedded shell image for its SHA-256, architecture and CRT imports. The
read-only `package-windows-shell.py image --installer INSTALLER.exe` entry repeats
that measurement for an existing immutable artifact. Its identity sidecar remains `awaiting_runtime` until the
required Windows 11 x64 and Server standard-user selections pass for that exact
installer. See the [native acceptance boundaries](windows-desktop-installation.md#evidence-and-release-claims).

The single source `apps/website/data/releases.json` uses V3. Website preparation
emits `releases-v3.json` for the Windows shell and a V2 `releases.json` projection
excluding every Windows artifact. Supported released macOS readers reject Windows
artifacts in V2, so that projection is retained in the compatibility register.

Each attempt is retained under `target/release-candidate/SHA/TARGET/ROUND/`.
`assets/` contains the DMG or archive plus companion manifest; `result.json`
records the full source revision, host, checks, timing, and the measured
`website_artifact` row. `build.log` retains subprocess output. No installer is
uploaded and no existing user installation is changed. Linux acceptance installs
into a temporary private HOME and starts/stops its own daemon inside an isolated
container; the HOME is removed afterwards. Logs and package evidence are retained.

## Hosted build and publication

Use the [HiRoute release skill](../.agents/skills/hiroute-release/SKILL.md#choose-acceptance-scope)
to select patch or minor acceptance and reuse existing evidence.

### Prepare before freezing the final candidate

Track release acceptance in [the release acceptance Issue](https://github.com/higress-group/HiRoute/issues/193).
Record the intended version, current main SHA, supported released source packages,
selected checks and unresolved native/upgrade gates there. Set the product version
through the shared tool and validate the resulting component and handshake
projections. Update both README languages, the website homepage/download page and
the public installation/Quickstart guides together. Linux's `standalone` package
kind includes its Web UI and CLI; public platform descriptions must reflect both.

Apply [website documentation maintenance](../apps/website/README.md#documentation-maintenance)
to the accumulated release diff: update affected bilingual guides and their
copyable Agent/CLI instructions, inspect product screenshots, and recapture changed
flows in dark mode. Record concrete reasons when existing text or captures remain
accurate. Finish this work as part of release preparation, not after publication.

Keep baseline results bound to their tested SHA while pending changes are reviewed.
After integration, assess the intervening production diff and run the affected
product smoke, including save/reopen, configured Gateway access and independent
restoration where applicable. Do not rerun unaffected broad suites solely because
documentation or version fields changed. Build every final package from the same
frozen source, and execute all outstanding package-bound/native gates against
those actual artifacts; an earlier baseline pass does not satisfy them.

### Write public release notes

Write the GitHub Release body and website catalog summary for people deciding
whether and how to upgrade. Cover user-visible changes, supported platforms,
and known limitations that affect normal use. Link to installation, upgrade,
and package verification instructions where applicable.

Keep build and publication steps, validation environments, passed or deferred
checks, and run links in the release Issue or PR. Review the public notes for
this separation before publishing or editing a release.

Prepare release notes separately from `apps/website/data/releases.json`. Add the public catalog
record only after exact package hashes and required acceptance are available.
Until then, distinguish upcoming platform capabilities from the versions actually
downloadable, and do not publish a Windows download row or model-feed availability
claim without its corresponding acceptance and publication prerequisites.

### Build and publish the frozen artifacts

Run the **build-release-candidates** workflow on the public candidate branch.
For a release with Omarchy packages, dispatch it from the frozen, reviewed
version commit on `main` so the protected signing workflow can admit the run.
Package builds and the reusable **windows-headless** workflow start alongside each
other at the same source commit. The **qualify** job requires every package build,
all five x64 native scopes and their selected Windows 11 x64-artifact replays.
It uploads `qualified-release-<sha>-<attempt>` with the exact public asset names,
sizes and digests. A failed or missing job prevents qualification. The recorded
producer attempts identify results reused after a partial retry of that run.

The public Windows asset is x64 Desktop. The internal ARM64 Rust installation
assessment remains in the default manual **windows-headless** run. All package
jobs use **build-release-platform** for their per-target build steps. Ordinary
iteration uses the [cross-compilation and development-machine path](validation-routing.md#windows-iteration-and-release-validation).
Hosted background compilation, logged-on-user acceptance and real accounts retain
their separate runtime gates.

Start [standard-user NSIS acceptance](validation-routing.md#development-machine-execution)
as soon as `build-windows-desktop` succeeds. Use the development bundle from that
same candidate run. Before publishing the GitHub Release, confirm the successful
qualification and native evidence identify the final installer's exact SHA-256.
If a later attempt qualifies a different installer, rerun the required native
cases on that installer.

For Windows-only development, dispatch **build-release-platform** on the exact
development ref with the default x64 target, Windows runner and
`windows_desktop: true`. It uploads the installer and build evidence with the
identity `awaiting_runtime`.

Every package matrix job checks out the dispatch's exact `github.sha`, uses a native
runner and uploads its own assets and evidence. Ubuntu runner labels select only
the native Docker host. Both Linux architectures build against Debian 11 / glibc
2.31 using the digest-pinned images and dated Debian and Debian-security package
snapshots in
[`linux-release-baseline.json`](../scripts/linux-release-baseline.json). The Rust
toolchain remains pinned by `rust-toolchain.toml`, Cargo stays locked, and Go uses
the pinned CPA source's toolchain declaration. Host Rust/Go tools are mounted into
the build image; actual tool versions are retained in provenance. sccache shares
compiler objects, while the checkout retains its default `target/`. The actual
build image digest and fixed recipe bind the target directory to its build
environment. Automatic BuildKit attestations are disabled for this local
toolchain image so repeated builds retain a stable digest across Docker image
stores; our recipe, image identity and measured toolchain provenance are retained.
An invalid image digest stops the build. A different baseline, an older target stamp or
pre-existing unattested release output requires a fresh worktree.
Both package sources use the same fixed timestamp. The security source supplies
matching libc build dependencies for the security-updated base image, including
the amd64 clang toolchain's `libc6-i386` dependency.
The container preserves an explicit positive `CARGO_BUILD_JOBS` from the caller;
otherwise it uses two compiler workers. Managed workbench callers must retain
their existing capacity reservation throughout the build.
Already-set standard proxy variables are forwarded by name to Docker's recipe
and compiler containers, including upper/lower-case HTTP, HTTPS, FTP, ALL and
NO_PROXY. Docker reads the values from the inherited client environment; values
are not command arguments or recorded provenance fields. The recipe uses Docker's
predefined proxy build arguments, which exclude them from image history/cache
metadata. Both native Linux build stages use the host network so existing
loopback proxies remain reachable. The minimum runtime has no proxy forwarding
and keeps network disabled.
The compiler's sccache server uses a Unix socket in its private container
filesystem, preserving the mounted object cache without contacting the host's
existing cache service. Host sccache IPC variables are not forwarded.
The mounted caller-selected sccache takes precedence over executables in the
mounted Cargo HOME, so server startup and compiler requests use the same tool.

The minimum distribution baseline is glibc 2.31 for **each** of x86_64 and aarch64.
The archive ABI gate checks every ELF payload (including CPA and any future
packaged dynamic library) after verifying the manifest, inventory and digests.
It rejects wrong architecture, unexpected loaders/libraries, build-host search
paths, newer loader tags, unknown symbol namespaces and symbol requirements above
the recorded GLIBC/GLIBCXX/CXXABI/GCC/OpenSSL floors. Static executables pass without
inventing a dynamic loader requirement. Library definitions are not confused with
required versions. A rejected file and symbol version appear in failure evidence.
The current package inventory contains no additional shared libraries; extending
that inventory still requires the existing packager/installer contract change.

A separate digest-pinned slim runtime image receives only the verified archive,
installer and test driver. It runs the production installer in a fresh HOME,
checks installed binary hashes, resolves dynamic dependencies eagerly, executes
CLI root help and CPA help, starts `hiroute service run`, verifies real
`system status` and `gateway show` replies twice across a resident daemon, and
requires both management replies to remain healthy in the second round, and
requires orderly SIGTERM shutdown with management and the Gateway listener no
longer available. Its disposable HOME contains Debug diagnostic settings; the
measured `level_applied` record must confirm Debug. Cleanup stops the process
before removing the HOME, retaining safe daemon JSONL on failures. Missing
runtime libraries fail here even if they existed in the build image. The runtime
has no external network and does not make model calls.

The CLI has no `--version` command. Release version identity comes from the
verified archive manifest and matching installation ownership marker, with
installed binary hashes checked against that manifest. Every real management
call also uses the production client's version handshake, which requires the
daemon and release versions to equal the CLI's compiled release version.

Linux installation creates missing state parents with mode 0700, including the
default `.local/state` chain and an explicitly selected absolute XDG state path.
It preserves safe existing ancestor permissions and refuses symlinks or writable
non-sticky ancestors, rather than changing the caller's umask or unsafe paths.
This keeps the production diagnostic writer usable under a group-writable umask.

`result.json` distinguishes the execution host, fixed build baseline and measured
per-architecture runtime result; `linux-abi.json` binds symbol evidence to the exact
archive and manifest hashes, source revision and target. Runtime verification also
matches the ABI baseline and installed payload hashes. The package remains
`awaiting_runtime` until the minimum environment
passes. A help-only probe, x86_64 success, or successful archive checks never establish
aarch64 runtime compatibility. Release acceptance additionally needs a protected
API-key model call on the intended deployment host, using the same frozen archive;
that live result is separate from the offline build gate. Rebuild after integrating
product fixes and retain each result at its actual source revision.

### Prepare Omarchy packages in Actions

After the candidate run succeeds, dispatch [Prepare Omarchy release](../.github/workflows/omarchy-release.yml)
on `main` with its `candidate_run_id`. The workflow uses the qualified Linux
archive and manifest from each producer's recorded attempt. Both native builders
use the stable Omarchy mirrors with package-signature checks. The ARM64 builder
also verifies the generic Arch Linux ARM root filesystem before importing it.

Configure an `omarchy-signing` GitHub environment restricted to the `main` branch
with a release reviewer. Set `OMARCHY_SIGNING_SUBKEY` to the armored OpenPGP
signing subkey export and `OMARCHY_SIGNING_PASSPHRASE` to its passphrase. Keep the
primary private key and revocation certificate offline. The signer checks both
package inventories, install permissions and embedded runtimes before importing the subkey into
a temporary keyring. It verifies the signatures against the frozen public key
and uploads `omarchy-signed` with the packages, signatures, checksums and identities.

The x86_64 job installs the exact signed package in Omarchy on GitHub KVM. It
uses the recorded stable predecessor to verify the native Wayland upgrade and
same-Plan Gateway receipts. Retain `omarchy-x86-kvm-evidence` with the release
review. On failure, rerun the failed jobs to reuse successful build or signing
artifacts.

### Run the local Omarchy smoke

After the hosted workflow succeeds, use the existing hardware-accelerated local
ARM64 Omarchy guest for one final graphical smoke. Read the host's private
`~/.config/hiroute/omarchy-smoke/config.json` or the path set in
`HIROUTE_OMARCHY_SMOKE_CONFIG`. Use mode `0700` for its private directory
and `0600` for the config and SSH files. Resolve relative SSH file paths from
the config directory. The version 1 format uses these fields with local values.

```json
{
  "schema_version": 1,
  "ssh": {
    "host": "127.0.0.1",
    "port": 2222,
    "user": "release-check",
    "identity_file": "id_ed25519",
    "known_hosts_file": "known_hosts"
  },
  "vm": { "disk_path": "/path/to/existing-omarchy-disk.img" },
  "capture": { "method": "grim" }
}
```

Confirm the running guest uses hardware acceleration and `vm.disk_path` and
that the configured SSH endpoint reaches that guest. Set the `ssh_*` variables
from the matching `ssh` fields, using the resolved paths. Connect with the
configured identity and host keys.

```sh
ssh -F /dev/null -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=yes -o GlobalKnownHostsFile=none \
  -o "UserKnownHostsFile=\"$ssh_known_hosts_file\"" \
  -i "$ssh_identity_file" -p "$ssh_port" -l "$ssh_user" "$ssh_host"
```

Keep guest startup, autologin and wake settings local to the smoke machine.
Confirm noninteractive package privileges with `sudo -k -n` and check for an
active, unlocked local Wayland session owned by the configured user. Keep the
smoke pending if the guest or its desktop is unavailable.

Use the `omarchy-signed` artifact from the successful x86_64 verification run.
In its `signed.json`, match `revision` to the frozen SHA and select the `aarch64`
package entry. Check the named package against its recorded `sha256` and verify
the detached signature using the release key at the frozen source. Close any
previous HiRoute Desktop window and confirm the old process has exited.
Install or update the verified package in the standard locations with the
[standard installation steps](linux-desktop.md). Open HiRoute from Apps and
complete the normal service setup or [update](linux-desktop.md#update-or-remove-the-background-service)
if prompted.

From SSH, bind `XDG_RUNTIME_DIR`, `HYPRLAND_INSTANCE_SIGNATURE` and
`WAYLAND_DISPLAY` to the active local session. Use `loginctl show-session` and
`hyprctl -j instances` to identify the session and its Wayland socket.
Bring HiRoute forward. Set `candidate_sha` to the frozen revision and
`capture_path` to a new PNG path in the private evidence directory. Run the
checks and capture in Bash.

```bash
(
  set -euo pipefail
  address=$(hyprctl -j activewindow | jq -er .address)
  desktop_pid=$(hyprctl -j clients | jq -er --arg address "$address" '
    .[] | select(.address == $address and .class == "ai.hiroute.desktop"
      and .mapped and .xwayland == false) | .pid')
  service_pid=$(systemctl --user show ai.hiroute.cli.service -p MainPID --value)
  [[ "$desktop_pid" =~ ^[1-9][0-9]*$ && "$service_pid" =~ ^[1-9][0-9]*$ ]]
  test "$(readlink "/proc/$desktop_pid/exe")" = /usr/bin/hiroute-desktop
  "/proc/$desktop_pid/exe" --version | jq -e --arg sha "$candidate_sha" '.source_revision == $sha'
  "/proc/$service_pid/exe" --version | jq -e --arg sha "$candidate_sha" '.source_revision == $sha'
  manifest=/usr/share/hiroute-desktop/standalone/manifest.json
  daemon_sha=$(jq -er '.files["bin/hirouted"].sha256' "$manifest")
  printf '%s  %s\n' "$daemon_sha" "/proc/$service_pid/exe" | sha256sum --check -
  umask 077
  set -C
  grim -t png - > "$capture_path"
)
```

Require the checks and capture to succeed. The running daemon digest must match
the packaged runtime even when a rebuild has the same source revision. Inspect
the new image to confirm the authenticated HiRoute console renders in the window.
Treat QMP `screendump` as a VM diagnostic on this GL display path. Add one
representative visual interaction when the release changes the UI.

Record a passing smoke with the candidate SHA, package SHA-256, guest version
and observed result in the release Issue or PR before publishing the Omarchy
assets. Keep captures, machine paths, launch settings and credentials in
private local storage. Shut down the test guest after the smoke.

### Publish the qualified artifacts

1. Require a successful qualification for the frozen SHA. Select the exact public
   assets named in that qualification. Keep the exact source
   commit available; internal master and an exported public commit may differ.
2. Download public package jobs' `assets/` and `result.json`. Keep Windows
   headless `internal-payloads/` out of the release. Use the measured
   `website_artifact` rows in a new release record in
   `apps/website/data/releases.json`, with the intended version/channel/date/notes.
   Add the finalized version to `apps/website/data/short-downloads.json` for
   [short website download links](../apps/website/README.md#public-release-contract).
   Preserve historical records and regenerate website content twice, checking
   that the second run introduces no difference.
   For Omarchy, use `HiRoute-<version>-<revision-prefix>-omarchy-<arch>.pkg.tar.xz`
   or the `.pkg.tar.zst` equivalent. Include the measured archive, `.sig` and
   `.sha256` records in the same catalog. The download page uses these records
   and the existing updater feed projections exclude them.
   Complete the [documentation review](../apps/website/docs/release-reviews/README.md)
   for the exact built product SHA and final website contents in this same PR.
   Run its `check` command plus website tests/build before publishing the GitHub
   Release. The publication workflow rejects a new release with a missing, pending,
   mismatched or stale guide/screenshot review before downloading its assets.
3. Merge the reviewed website manifest into public `main`, tag the exact built
   source SHA, and attach the exact candidate assets to that GitHub Release.
   The existing [publish-release workflow](../.github/workflows/release.yml)
   matches the catalog's Windows release assets to a qualification from a successful
   candidate workflow, then verifies the downloaded bytes and package identities.
   After those checks it publishes OSS/site links and proposes the
   [release contract snapshot](release-contracts.md). Rerunning publication for a
   recorded release uses its exact immutable snapshot when the candidate artifacts
   have expired. A changed source, channel or asset identity requires qualification
   for the new candidate.
   Windows Desktop also attaches its `INSTALLER.exe.identity.json`; publication
   verifies the installer digest, source revision and native case evidence. Complete
   acceptance is the default. An explicit maintainer-directed deferral must match
   the exact version/source/installer in the existing Windows acceptance policy,
   retain `not_assessed` verdicts, and still prove standard-user installation and
   console readiness for both selections. A mismatched or undeclared omission fails.

The build workflow only stores Actions artifacts. It does not publish a GitHub
Release, alter the stable catalog, or replace any immutable distribution object.
