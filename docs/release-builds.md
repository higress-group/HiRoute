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
sccache and native build tools (including `readelf` on Linux).
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
uploaded, no user installation is changed, and no product service is started.

## Hosted build and publication

Run the **build-release-candidates** workflow on the public candidate branch.
Every matrix job checks out the dispatch's exact `github.sha`, uses a native
runner and uploads its own assets and evidence. Linux uses Ubuntu 22.04 to avoid
implicitly raising the distribution's glibc baseline to a newer runner image.
The Linux result records ELF dependencies and symbol versions for checking the
deployment image's library requirements.

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
