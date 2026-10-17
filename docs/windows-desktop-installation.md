# Windows Desktop installation and updates

The [English installation guide](../apps/website/content/guides/install-windows.en.md)
and [Chinese installation guide](../apps/website/content/guides/install-windows.zh.md)
own the user steps, SHA-256 verification and SmartScreen prompts. The NSIS package
supports native Windows x64 and installs for the current ordinary user without
Python, WSL or administrator privileges.

The shell's program directory is fixed at `%LOCALAPPDATA%\Programs\HiRoute`.
Standalone service state remains under `%LOCALAPPDATA%\HiRoute\cli`; neither root
can contain the other. The native adapter checks KnownFolders, architecture,
existing manager ownership and reparse-free paths before product writes. An
SCM-managed installation is refused unchanged because SCM installation is not
offered in this release.

The installer verifies the complete embedded standalone package and uses its
native Rust transaction, the single Windows installation producer. The core
requires the package target to match the native host; the NSIS package and public
Desktop target alone are x64-only. Only the current
`hiroute.windows-install-journal/v1` schema is read. An unreleased Task installation
is adopted when its layout and journal match; incompatible material requires
repair/reinstallation. A prepared journal blocks business startup until recovery reconciles
files, the marker and actual Task registration. No new business mutations are
replayed during installation recovery.

The separate shell inventory records only its installed files and digests. Its
private preparation journal retains matching previous bytes until installation,
login settings and actual Control/Gateway readiness succeed. Changed/foreign
entries are preserved and require repair. Committed cleanup does not replay
installation or roll back a successfully committed package.

Recovery and uninstall run a verified shell copy outside the program directory.
A failed first installation can therefore roll back before `shell-package.json`
exists, without trying to delete a running shell or CLI. When no Agent storage or
storage-upgrade backup exists, rollback does not need to start the daemon.

After both transactions commit, the Rust core prunes obsolete versioned payloads
under `%USERPROFILE%\.local\lib\hiroute`. It retains the previous package until
shell rollback is no longer possible. Uninstall uses the bundled CLI outside the
version root and removes the verified current payload too. Cleanup resumes from
the current manifest after interruption and preserves changed, foreign or linked
entries. The shell records cleanup results in
`%LOCALAPPDATA%\HiRoute\shell\package-cleanup.json` for diagnosis.
After uninstall commits, package cleanup errors are reported without undoing the
uninstall or skipping an explicitly requested removal of standalone data.

The installer accepts `/S` for unattended installation with the default login-start
and tray choices and no prompts. Failures return a nonzero process exit code.

Login-start and tray choices default on and can be deselected independently.
Login start updates the user's Task and the fixed HKCU Run command together,
compensating a failed Run update. Disabling the tray makes window close exit the
shell; a background login launch then opens a visible window. Quit never stops the
service. A second launch focuses the existing window.

## Update ordering and recovery

The host selects a newer stable x64 NSIS entry from `releases-v3.json`. The page
cannot supply a download URL, install directory or executable. Downloading and
size/SHA-256 verification finish in private staging while the service runs. Native
lifecycle preparation pauses new admissions and reports active calls/tasks. The
user can cancel; cancellation resumes admission and leaves work untouched.

After drain, a private native finisher acknowledges readiness and requires a
generation-bound explicit commit. It waits for the shell to exit, rechecks the
staged installer and runs NSIS passively with its update
flag. The installer repeats drain before stopping the owned daemon. A manual
installer uses that same hook and a bounded wait. An older daemon without drain
support requires a separate explicit interactive confirmation; passive updates
refuse it.
Passive updates skip the login/tray options page and reuse the saved choices;
silent `/S` installations continue to use the documented defaults.

Pre-stop failures resume admission. Replacement failure attempts the old package
and Task through the same Rust journal core; it never rewrites a marker to pretend
recovery succeeded. The next shell start reads the private outcome and can recover
an interrupted transaction after the installer is no longer active. An unresolved
file or service conflict stays on the bundled failure page with a diagnostics path.

## Uninstall boundaries

Agent restoration uses production Preview/Apply and waits for the resulting
Operation before service or file removal. A blocked restore reports the Agent IDs
and leaves the installation available for a user decision. The Task, exact owned
shell login command and recorded files are removed; a foreign Run command or file
is preserved. User PATH removal applies only when the installer added that fixed
entry. Data removal is an explicit unchecked option and does not turn the headless
state root into the NSIS program directory.

Codex's model profile and original root are distinct restore contexts. Its profile
status also displays the root's collaboration state; uninstall restores that
Skill through the root and restores any model profile separately. A Skill ownership conflict
still blocks uninstall before any restore is applied.

If the service cannot start or Agent state cannot be enumerated, interactive
recovery/uninstall offers a separate confirmation with **No** selected by default.
It explains that configured Agents may need manual restoration, affected Agents
cannot be listed, and stopping the owned unresponsive Task may interrupt work.
Continuing always retains all business data, including restoration material.
Silent/passive mode refuses this exception; a known restoration conflict cannot
be bypassed by it. Retry the installer to recover an interrupted installation.

## Evidence and release claims

The native acceptance inventory is
[`windows-desktop-cases.json`](../scripts/windows-desktop-cases.json). Windows 11
x64 and Server selections require real standard-user execution, the candidate SHA,
installer digest, OS/build and architecture. A Server result is not Windows 11
evidence. Browser fixtures, Cargo checks and package construction do not establish
installation, native authorization, upgrade recovery or a live model call.

The [release builder](release-builds.md) keeps a Desktop installer separate from
the headless archive. Release publication requires both native selections and
refuses an identity sidecar that is incomplete or bound to another artifact.
