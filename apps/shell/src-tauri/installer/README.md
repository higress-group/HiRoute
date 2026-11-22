# Windows installer template

The template is derived from [Tauri CLI 2.11.4](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi), under the accompanying MIT license. The upstream source SHA-256 is `20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079`. Keep the npm CLI pin and template aligned.

Silent installation applies the default login/tray choices, skips interactive
process prompts and uses explicit default responses for failure messages. Runtime
acceptance must still demonstrate `/S` with the actual NSIS package. Passive
updates skip the options page and reuse saved preferences. Hooks are included
after the template declares the variables used by their page functions.

The options page offers a program folder on first install alongside login and tray preferences. The native transaction checks the destination before NSIS writes the payload. It binds recovery to the prepared directory and subsequent maintenance to the current-user Installed Apps registration. The installed program, background service and Agent restoration transactions retain their separate file inventories.

Recovery and uninstall extract the package's matching shell helper into the private NSIS plugin directory.
Unavailable Agent state has a separate default-No confirmation; silent/passive
runs refuse it and exceptional removal always keeps business data. Package cleanup
waits for both transaction owners to commit and preserves rollback sources.

The preinstall helper also receives the manifest-bound candidate daemon extracted
into the private plugin directory. It checks storage prerequisites before overlay.
Only a preflight requiring format-changing history cutover retains the complete
previous Desktop/native installation and fixed HKCU registration; ordinary current
repair avoids this archival work. It accounts for simultaneous allocations on their actual native volumes. The
interactive history notice precedes replacement; silent/passive callers receive
the corresponding notice through installation help or the originating Updates UI.
The generated whole-group guide uses the installation owner's file map and actual
Task/login choices, including separate choices, rather than reconstructing paths.

The first supported Windows release has no supported published predecessor.
Generic complete recovery remains; development historical compatibility promises
and exhaustive development upgrade matrices are outside this change.


Silent uninstall tooling must wait for the NSIS worker and check actual removal.
The registered quiet command, supported synchronous invocation, data-preservation
behavior and latest worker outcome are specified in the
[Windows installation guide](../../../../docs/windows-desktop-installation.md#scripted-silent-uninstall).
