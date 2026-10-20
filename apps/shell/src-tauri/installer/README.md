# Windows installer template

The template is derived from [Tauri CLI 2.11.4](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi), under the accompanying MIT license. The upstream source SHA-256 is `20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079`. Keep the npm CLI pin and template aligned.

Silent installation applies the default login/tray choices, skips interactive
process prompts and uses explicit default responses for failure messages. Runtime
acceptance must still demonstrate `/S` with the actual NSIS package. Passive
updates skip the options page and reuse saved preferences. Hooks are included
after the template declares the variables used by their page functions.

Local changes enforce native x64 and Task-only admission before product writes, fix the program directory under LocalAppData/Programs, replace directory selection with login/tray choices, preserve the headless service during shell upgrades, and delegate optional data removal to the Rust Agent-restore/journal transaction. The uninstaller deletes its inventoried files and empty directories; it never recursively removes an application/state directory.

Recovery/uninstall use an exact shell copy in the private NSIS plugin directory.
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
