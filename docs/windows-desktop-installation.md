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
After NSIS overlays a replacement, obsolete owned frontend asset paths are removed
only when both their current bytes and retained backups match the old inventory.
This happens before the new payload's strict closed-file-set verification. A retry
recovers any pending shell transaction first. If the standalone core still matches
the previous installation, shell rollback restores its captured settings and files
without reinstalling or draining that unchanged core.
Once both owners commit, the shell atomically records the successful outcome before
retiring its journal. A successful manual repair replaces an earlier failure in
startup and Settings; committed-phase recovery safely repeats that reporting step.

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

## Local console unavailable

The service binds its console at startup. Disabling the console with
`hiroute console disable` or `hiroute console init --mode off` therefore prevents
the shell from opening it after the next service restart. An occupied explicit
console port has the same effect, while Control and Gateway remain independent.
The bundled page identifies this local-console failure and shows the log path.
Recheck does not change the saved configuration or create a temporary listener.

To restore the local interface with automatic port selection, run in PowerShell:

```powershell
hiroute console init --mode loopback
hiroute service restart
```

Restart can interrupt active calls and tasks. After it completes, select Recheck
in the shell. Keeping the console disabled remains an explicit owner choice.

## Update ordering and recovery

The host selects a newer stable x64 NSIS entry from `releases-v3.json`. The page
cannot supply a download URL, install directory or executable. Downloading and
size/SHA-256 verification finish in private staging while the service runs. Native
lifecycle preparation pauses new admissions and reports active calls/tasks. The
user can cancel; cancellation resumes admission and leaves work untouched.

A missing or retired Windows catalog (HTTP 404/410) is shown as an unavailable
update check. The current installation and service remain usable; retry later.
The shell does not call that a failed installation, claim the current version is
latest, or use the macOS V2 feed as a Windows fallback.

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

When starting a stopped service, the shell also handles a failed CLI readiness wait.
It captures a diagnostic watermark before the start request and accepts only a new
failed startup record. Old, incomplete or unreadable diagnostics leave the manager
error intact. The typed current cause can explain a lock, missing key or upgrade
refusal without borrowing an earlier batch's advice.

## Windows support and acceptance scope

This is the first supported Windows release, with no supported released Windows
predecessor. General complete-group recovery, matching programs/native-state
preservation and the existing transaction safety boundaries remain available.
Their presence is not a historical compatibility promise. Development builds and
simulated cross-build replacements are not released-version upgrade evidence;
expanding their historical upgrade/restore matrix is separate follow-up work.

Current installation, failure/interruption repair, actionable errors and data
protection remain required. Ordinary current-format repair does not duplicate the
complete Desktop or export Task/Apps registration merely to support a historical
simulation. Those complete-source operations are used when authoritative candidate
preflight actually requires a format-changing history cutover. Retained recovery
material is never deleted to narrow acceptance.

## Storage-format failure and whole-group recovery

Before NSIS replaces installed files, its verified candidate daemon checks the
stopped-source requirements. When an actual history cutover is required,
preparation retains both the complete previous
Desktop (including shell, uninstaller and inventory) and standalone version root,
the native transaction's CLI/daemon/Skill/documentation file map, owned native Task XML and definition,
login choices and the fixed current-user HiRoute Apps registration. The protected
handoff binds this material to the storage root and new daemon image. Space checks
combine simultaneous allocations on the same native volume and check separate
program/data volumes separately. Candidate preflight runs before replacement; startup
revalidates under the storage writer barrier. Package cleanup retains the old
source while its handoff is needed. This does not use macOS App discovery or
assume that a PowerShell variable is inherited by the logged-on Task.

The bundled failure page displays the storage-owned reason and recovery action
even when the daemon and console cannot start. It distinguishes missing/invalid
packages, low space, unavailable keys and unknown state. Fix a stated prerequisite
before retrying. After a completed readiness wait, it uses the latest typed daemon
startup failure; a dependency or Gateway failure cannot borrow an old storage
report. Installer, manager-command and console failures have their own guidance.
An incomplete installation is identified independently of service readiness and
offers the exact installer repair path, without an unproven storage retry or old
batch location. A verified resumable/restorable source shows its original batch
ID and relative source directory, including long names without clipping. From the
parent of the displayed local log directory, open that source directory and read
`RECOVERY.md` or `恢复说明.md`, then `PROGRAM-LOCATIONS.md` where present. Missing,
unsafe or unverified batch locations must not be presented as verified material.
Resume an admitted batch with the current program; never start
an old program against newer/mixed data. Shell rollback checks storage cutover
facts before reinstalling or starting an old service. An uncommitted shell journal
is not proof that business data is unchanged.

If an upgrade starts the new service but cannot finish installation, keep both
complete program versions, all data and the installation journals. Close files
reported as in use and check access and free space, then rerun the **same complete
current installer**. Its early recovery verifies the accepted payload before a
forward repair; it does not replace the original recovery batch or launch the old
daemon over current data. A changed package or native file is refused. If repair
still cannot be verified, preserve the material and use the verified batch's
whole-group guide or obtain support. Check actual service/Gateway readiness and a
request after repair; do not infer recovery from installer or Task exit alone.

When complete restoration is required and the source is verified, stop the owned Task/daemon and automatic starts, keep
newer storage with its matching program separately, and follow the exact verified
batch's Chinese/English generated guide. Restore all stores, keys, Observation and
recorded auxiliary locations together, then the matching full package and Task
registration through the platform installation path. Verify actual service and
Gateway readiness and a request. Missing keys or a substituted/incomplete source
cannot be repaired by blind retry or choosing another old release. Inspect safe
startup diagnostics and obtain support while retaining every generation.

Use `hiroute service recovery --output json` for an offline report. Verified
resume/restore guidance includes the exact batch and absolute source directory,
derived from the actual storage root. Windows Server
and Windows 11, ordinary-user and administrator sessions, and released versus
development packages retain separate evidence boundaries.

### Chinese recovery quick guide

本次是 Windows 首个受支持版本，没有已发布旧版的历史兼容承诺。通用完整恢复能力和数据保护仍保留；开发构建模拟不等于正式升级支持。
安装或启动失败时，先保留完整程序、全部数据、密钥、安装记录与恢复包。
文件占用、权限或空间问题应按当前提示处理后再试；不要清空历史、批量修改权限或删除安装记录。
如果新服务已启动但安装尚未完成，解除占用并核对权限、空间后，重新运行**同一当前版的完整安装器**。
安装器会核对原先接受的程序包与任务注册，再继续完成安装；程序或记录不符时会拒绝，不能换任意旧安装器覆盖。
修复之后检查 `hiroute system status --output json` 中的服务与 Gateway 实际就绪状态，并完成一次真实请求。

服务与控制台无法启动时，使用安装包自带的故障页面，或离线运行 `hiroute service recovery --output json`。
已验证的恢复材料会给出原批次 ID、源目录与说明文件名；从页面所示日志目录的上级目录定位该源目录，
先读该批次的 `恢复说明.md` 或 `RECOVERY.md`，再读 `PROGRAM-LOCATIONS.md`。
整组恢复前停止当前账户的 Task、守护进程、CPA 与自动启动，将新版完整数据和配套程序另存，
再按记录的位置与权限还原旧数据、全部密钥、历史、完整程序和原 Task 注册，并核对原登录启动选择。
不要逐库覆盖、混用不同批次，或让旧程序读取新版数据。恢复后仍需检查旧配置、路由、历史与一次真实请求。
缺少密钥、完整原版程序、有效批次，或无法核对文件归属时，保留现场与诊断并联系支持；反复重试或任意降级不能补齐这些材料。

## Uninstall boundaries

Agent restoration uses production Preview/Apply and waits for the resulting
Operation before service or file removal. A blocked restore reports the Agent IDs
and leaves the installation available for a user decision. The Task, exact owned
shell login command and recorded files are removed; a foreign Run command or file
is preserved. User PATH removal applies only when the installer added that fixed
entry. Data removal is an explicit unchecked option and does not turn the headless
state root into the NSIS program directory.

Restoration includes each independently managed model, collaboration Skill and
management Skill. Codex's model profile and original root are distinct contexts:
profile status displays both root-owned Skill facets, so uninstall restores those
through the root and any model profile separately. Explicit native profile targets
from discovery remain bound to status, Preview and Apply. Incomplete or conflicting
managed state blocks uninstall before any restore is applied.

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
