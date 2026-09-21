# HiRoute distribution candidate component notice

[简体中文](DISTRIBUTION-NOTICE.zh-CN.md)

The HiRoute Rust workspace is licensed under Apache-2.0. The package contains HiRoute
Desktop, `hirouted`, `hiroute`, frontend resources, the built-in model catalog, and a pinned
CLIProxyAPI (CPA) version. Each component retains its own version and license. CPA's version,
source commit, and final signed-byte digest are recorded in `installation.json`.

Declared licenses for third-party Rust/npm dependencies are listed in
`dependency-inventory.json`. `Licenses/THIRD-PARTY-LICENSES.txt` and its JSON manifest are
generated deterministically from locked dependencies and the pinned CPA commit. They include
the LICENSE, NOTICE, and related materials distributed with each component; the CPA and
HiRoute licenses are stored in the same directory. The tooling never substitutes HiRoute's
license for a third-party license and refuses to package incomplete materials.

`controlled-trial` means only that the package is self-signed and not notarized. It does not
waive any license-material requirement.
