# Windows console shell

This x64 Tauri host displays the daemon's shared Web console. Product state,
authentication, confirmations and operations stay in `hirouted` and
`crates/product-session`; there is no second Tauri product-command bridge.

`src-tauri/src/runtime` owns these boundaries:

- `service`: fixed installed CLI lookup, bounded native execution, real
  Control/Gateway readiness and one-time console launch minting.
- `mod` / `commands`: private WebView, single instance, navigation lock and
  explicit host commands, each checked against the exact active console origin.
- `tray` / platform `shell_login`: window visibility and current-user login start;
  quitting the shell leaves the Task running.
- `installer`: NSIS's fixed adapter to the native Rust installation transaction.
- `install_transaction`: shell-only digest inventory, prepared/committed backup
  record and failure recovery; standalone journals keep their separate owner.
- `updater`: fixed HTTPS V3 selection/download, verification, cancellable drain
  and a private native finisher that waits for shell exit before passive NSIS.

`src/main.ts` and `index.html` are the bundled startup/failure page. They use the
Desktop production styles and never render caller-supplied HTML. The running
console uses `apps/desktop/src/host`'s `shell-console` transport: product commands
use authenticated HTTP; only the explicitly listed host actions use IPC.

The remote capability matches loopback HTTP; Rust also checks the exact current
origin/port on every host call. It grants no filesystem, shell, product-command,
generic plugin or Local Control access. Tauri's native postMessage fallback keeps
host IPC available under the console's strict HTTP CSP.

Build the installer through `scripts/build-release.py --windows-desktop` on the
native x64 Windows producer. Silent NSIS installation (`/S`) applies default
login/tray choices with no prompts. This is the only offered Windows install
package; headless archives stay internal and SCM installation is not offered. It stages a verified standalone package under the
ignored `src-tauri/resources/standalone` directory. Do not hand-populate or publish
that staging directory. The vendored NSIS template and hooks are explained in
[`installer/README.md`](src-tauri/installer/README.md).

See [installation and recovery](../../docs/windows-desktop-installation.md) and
the [native acceptance boundaries](../../docs/windows-desktop-installation.md#evidence-and-release-claims). Browser
screenshots and compilation do not replace native standard-user acceptance.
