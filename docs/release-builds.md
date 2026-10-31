# Release candidate builds

The candidate entry builds macOS Desktop DMGs (`arm64`, `x86_64`) and Linux or Windows
headless archives (`x86_64`, `aarch64`) and a separate x64 Windows Desktop NSIS
installer. Windows headless archives are internal installer/test payloads only;
Windows publication consists solely of the x64 NSIS installer. A target's package integrity is separate from its required native runtime
acceptance and does not by itself authorize publishing that target. All components of one release must come from the same clean,
committed source revision, with CPA built from `vendor/cpa/source.json` and its
checked patch.

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

Run the **build-release-candidates** workflow on the public candidate branch.
Unix candidate builds start independently of the reusable **windows-headless**
workflow at the same source commit. All five native scopes, their selected Windows 11
x64-artifact replays and the native ARM64 Rust-installation job must succeed before Windows package-build jobs can start.
Both groups use **build-release-platform** for the same per-target build steps. This expensive Windows matrix
does not run automatically on PR or main pushes; ordinary iteration uses the
[cross-compilation and development-machine path](validation-routing.md#windows-iteration-and-release-validation).
Hosted background compile-only cases and real accounts retain their separate
runtime gates; the CI dependency does not establish those results.

To prepare only an x64 NSIS installer for development-machine acceptance, dispatch
**build-release-platform** on the exact candidate branch with its default x64
Windows target, Windows runner and `windows_desktop: true` option. It uploads
the installer and build evidence, and leaves the identity `awaiting_runtime`.
It performs no publication and does not replace the required native acceptance
or the release-candidate matrix.

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

1. Require the Windows validation gate and every selected package job to succeed
   for the same SHA. Keep the exact source
   commit available; internal master and an exported public commit may differ.
2. Download public package jobs' `assets/` and `result.json`. Keep Windows
   headless `internal-payloads/` out of the release. Use the measured
   `website_artifact` rows in a new release record in
   `apps/website/data/releases.json`, with the intended version/channel/date/notes.
   Preserve historical records and regenerate website content twice, checking
   that the second run introduces no difference.
3. Merge the reviewed website manifest into public `main`, tag the exact built
   source SHA, and attach the exact candidate assets to that GitHub Release.
   The existing [publish-release workflow](../.github/workflows/release.yml)
   verifies tag/package identity and digests, then publishes OSS/site links and
   proposes the [release contract snapshot](release-contracts.md).
   Windows Desktop also attaches its `INSTALLER.exe.identity.json`; publication
   verifies the installer digest, source revision and complete native case evidence.
   An incomplete or misbound sidecar cannot publish a Windows Desktop artifact.

The build workflow only stores Actions artifacts. It does not publish a GitHub
Release, alter the stable catalog, or replace any immutable distribution object.
