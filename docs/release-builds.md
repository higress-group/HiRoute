# Release candidate builds

The release scope is two macOS Desktop DMGs (`arm64`, `x86_64`) and two Linux
headless archives (`x86_64`, `aarch64`). Windows and Linux Desktop are outside this
build entry. All components of one release must come from the same clean,
committed source revision, with CPA built from `vendor/cpa/source.json` and its
checked patch.

The candidate builder must reject a host/target mismatch, dirty source, external
Cargo targets, inconsistent component versions and wrong-architecture binaries.
It must retain a separate output directory for every attempt, use checkout-local
`target/` and the existing shared build lock, and preserve logs on failure.
The existing website release manifest remains the sole publication catalog;
candidate metadata is input for review, never an automatically published release.

## Build locally

Use the intended native platform with the repository Rust toolchain, Node 22+
(Desktop and packaging regression checks), Go matching the pinned CPA source,
sccache and native build tools. Linux additionally requires Docker: the same
command enters the pinned glibc 2.31 build image and then a separate minimal
runtime image. Host distribution libraries are never used as the release ABI
baseline. The native architecture check applies to the host and both containers.
The CPA checkout must contain the commit in `vendor/cpa/source.json`; its current
branch does not select the source. Invoke from the clean candidate checkout:

```sh
python3 scripts/build-release.py \
  --target x86_64-apple-darwin --cpa-source-repo /path/to/CLIProxyAPI

python3 scripts/build-release.py \
  --target aarch64-unknown-linux-gnu --cpa-source-repo /path/to/CLIProxyAPI
```

The same entry accepts `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`.
macOS candidates keep the existing macOS 15 minimum and self-signed trial
distribution. Developer ID signing and cross-building on a Mac remain available
through [package-desktop.py](../scripts/package-desktop.py) as documented in the
[macOS installation guide](macos-installation.md); the matrix entry uses native
hosts so its probes execute on the selected architecture.

Each attempt is retained under `target/release-candidate/SHA/TARGET/ROUND/`.
`assets/` contains the DMG or archive plus companion manifest; `result.json`
records the full source revision, host, checks, timing, and the measured
`website_artifact` row. `build.log` retains subprocess output. No installer is
uploaded and no existing user installation is changed. Linux acceptance installs
into a temporary private HOME and starts/stops its own daemon inside an isolated
container; the HOME is removed afterwards. Logs and package evidence are retained.

## Hosted build and publication

Run the **build-release-candidates** workflow on the public candidate branch.
Every matrix job checks out the dispatch's exact `github.sha`, uses a native
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

1. Require all four matrix jobs to succeed for the same SHA. Keep the exact source
   commit available; internal master and an exported public commit may differ.
2. Download each job's `assets/` and `result.json`. Use the measured
   `website_artifact` rows in a new release record in
   `apps/website/data/releases.json`, with the intended version/channel/date/notes.
   Preserve historical records and regenerate website content twice, checking
   that the second run introduces no difference.
3. Merge the reviewed website manifest into public `main`, tag the exact built
   source SHA, and attach the exact candidate assets to that GitHub Release.
   The existing [publish-release workflow](../.github/workflows/release.yml)
   verifies tag/package identity and digests, then publishes OSS/site links and
   proposes the [release contract snapshot](release-contracts.md).

The build workflow only stores Actions artifacts. It does not publish a GitHub
Release, alter the stable catalog, or replace any immutable distribution object.
