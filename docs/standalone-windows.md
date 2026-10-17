# HiRoute on Windows

The Windows x64 Desktop installer is the only supported Windows installation
path in this release. It includes the CLI, daemon, CPA, Skills, Web console and
Desktop shell. A standard user needs neither Python, WSL nor administrator rights.
Users who only need the CLI and daemon install the same package and can quit the
shell afterwards; the background service keeps running.

## Install

Download `HiRoute-<version>-<rev12>-windows-x86_64-setup.exe` from the
[download page](https://hiroute.ai/en/download/) when a validated release is
available. Follow the [Windows Desktop guide](windows-desktop-installation.md)
for SHA-256 verification and the unsigned publisher/SmartScreen prompt.

Run the installer in your ordinary logged-on account. Login start and tray
residency are enabled by default and can be deselected independently. The shell
opens after the service is ready. The installer adds the fixed user CLI directory
to PATH; open a new terminal afterwards:

```powershell
hiroute system status --output json
hiroute service status --output json
hiroute console open
```

For an unattended installation, verify the installer first, then run the same
package with `/S`. This uses the default choices without prompts. The process
exit code reports failure; a successful install starts a healthy service.

```powershell
$process = Start-Process -FilePath .\YOUR-INSTALLER.exe -ArgumentList '/S' -Wait -PassThru
$process.ExitCode
```

The NSIS installer only supports native x64 hosts and refuses other architectures
before product writes. Windows headless archives for x64 and ARM64 are internal
build/test payloads, not downloads or a separate supported installation path.

## Service and console

Task Scheduler runs the service as the current logged-on user with an
InteractiveToken and least privilege. Ordinary start and stop require no account
password or elevation. Closing the launching terminal does not stop it; logging
off ends its session. Starting now and starting at login are separate controls:

```powershell
hiroute service start --output json
hiroute service stop --output json
hiroute service autostart enable --output json
hiroute service autostart disable --output json
hiroute service doctor --output json
hiroute service logs --output json
```

The Task and Desktop use the same instance, data and Gateway. The shell's login
preference manages both the Task and its own login entry. The authenticated
[Web console](web-console.md) can also be opened in a browser.

SCM service installation is not offered. Installation, update and uninstall refuse
an existing SCM-managed installation before modifying it and explain the refusal.

## Ownership, updates and removal

Windows KnownFolders determine the actual account Profile and LocalAppData;
HOME and XDG variables cannot redirect an installed instance. Shell program files
live in `%LOCALAPPDATA%\Programs\HiRoute`. Standalone state lives separately in
`%LOCALAPPDATA%\HiRoute\cli`; versioned CLI/daemon/CPA files and shared data remain
under the user's `.local` directories. The install directory never contains or
sits inside the standalone state directory.

The Rust installation transaction verifies the complete inventory, revision,
digests and executable architecture before publishing stable entries. The package
target must match the native host. An existing unreleased Task installation is
adopted only when its layout and journal match the current schema; incompatible
or changed material requires repair/reinstallation and is preserved for diagnosis.
There are no released Windows Python-installer or ARM64 installations to migrate.

Updates from Settings or a manually downloaded newer installer use the same
journaled transaction. Active calls and accepted Worker tasks drain before stop;
cancelling resumes admission without killing or replaying work. Rerun the
installer after interruption. A prepared `hiroute.windows-install-journal/v1`
blocks business startup until files, record and actual Task registration agree.

Use **Installed apps → HiRoute → Uninstall**. HiRoute first restores managed Agent
access through its existing restore flow. Conflicts stop removal and list the
Agents that need a decision. Uninstall removes owned program files, Task, shell
login entry and the CLI PATH entry it added. Data is preserved by default. Explicit
data removal affects only standalone data/state, never Agent files or projects.

If the service is unavailable, the interactive installer or uninstaller can offer
an explicit, default-No choice to continue without verified Agent restoration.
Configured Agents may need manual repair and stopping the owned Task may interrupt
work. Continuing retains all data; unattended mode refuses this exception. An
enumerated restoration conflict still stops removal. A first-install rollback
with no Agent storage does not require a running service.

After a successful upgrade, obsolete verified version directories are removed
once shell rollback no longer needs them. Uninstall removes the current verified
version too. Changed or foreign entries are preserved for repair.

## Native evidence

[Validation routing](validation-routing.md#windows-iteration-and-release-validation)
owns native execution. Record the actual source SHA, artifact hashes, OS/build,
processor and account/session. Windows Server x64, Windows 11 x64 and native ARM64
are separate evidence. The Windows 11 ARM CI installation case uses an ARM64
package through `__windows-install` / `__windows-uninstall`; internal test payloads
do not establish a released ARM64 Desktop. Retained SCM kernel/daemon/CLI code and
low-level tests do not establish a supported SCM installation flow.

Real native clients and provider authorization require their own execution.
Fixtures, compilation, manager registration and action launch cannot prove a
healthy Gateway, installation, upgrade or real model call.
