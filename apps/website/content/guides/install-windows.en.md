# Install HiRoute Desktop on Windows

HiRoute 0.5.0 introduces Desktop for native Windows x64, using a current-user NSIS installer. It includes the command line, background service, console files and a WebView2 bootstrapper. Python and administrator privileges are not required. Choose a Windows Desktop installer from the [download page](/en/download/) when a validated release is available.

## Verify the download

The filename is `HiRoute-<version>-<revision>-windows-x86_64-setup.exe`. Download from `hiroute.ai/releases/<version>/` and compare the full SHA-256 shown beside that exact file:

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath "$env:USERPROFILE\Downloads\YOUR-INSTALLER.exe"
```

[Get-FileHash](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-filehash) calculates the file digest. If it differs, delete the file and download it again.

## Open an unsigned installer

This release is unsigned. Windows may show an unknown publisher or a SmartScreen reputation warning. After verifying the source and digest, select **More info → Run anyway** when that choice is offered. An organization policy or Smart App Control may block this choice; use your organization's approval process. Do not disable Windows security. Microsoft describes these protections in [App & browser control](https://support.microsoft.com/en-us/windows/security/windows-security/app-browser-control-in-the-windows-security-app).

## Install and start

1. Open the installer in your ordinary account. It installs in `%LOCALAPPDATA%\Programs\HiRoute` and keeps service data separately under `%LOCALAPPDATA%\HiRoute\cli`.
2. Login startup and the tray are enabled by default. You can deselect either option. With the tray disabled, closing the window exits the shell; the background service continues.
3. Open HiRoute. It checks the current-user Task, starts the service, waits for the Gateway and opens its local console. There is no password or unlock step. Protected actions retain their page confirmation.

An existing current-user Task installation is upgraded. SCM installation is not offered in this release. An existing SCM-managed installation is refused unchanged before product writes. ARM64 and x86 hosts are unsupported by this Desktop installer.

The installer adds the fixed `.local\bin` directory to your user PATH without replacing other entries. Open a new terminal for the updated PATH. Settings and the tray's **Start at login** action manage the shell and Task together. **Quit** exits the shell and leaves the service running.

For unattended installation, run the verified installer with `/S` and wait for its process exit code. It applies the default login-start and tray choices without prompts, starts the service and returns a nonzero code on failure. CLI-only users install this same package. Windows standalone archives and ARM64 packages are not published.

## Update and recover

Open **Settings → Updates**. The shell downloads a newer stable installer from the official HTTPS catalog and verifies its size and SHA-256 while your service keeps running. Installation waits for active calls and Worker tasks. **Cancel update** resumes normal admission without cancelling or replaying your work. The installer then replaces the application and reopens HiRoute.

A manually downloaded newer installer uses the same drain and transaction path. An older daemon that cannot drain requires an explicit interactive confirmation before it can be stopped; a passive update refuses it. If an update fails, the startup page or Updates shows the failure and recovery status. Keep the logs and retained backups. Starting the shell after an interrupted installer attempts journal recovery and resumes paused admission; changed or foreign files require repair and are preserved.

This is the first supported Windows release. Generic complete-group recovery remains available when verified material is required; this does not promise compatibility with an unpublished development build. Development historical-upgrade matrices are outside this change. Current-format installation/repair does not export Task/Apps registration or duplicate the complete Desktop unnecessarily.

Before a storage-format change, configuration, credentials and routing continue, while old HiRoute observations are sealed with the matching old program. Current history/statistics start at cutover; old-period continuation/history references require complete-group matching-version recovery. Native Agent history is unchanged. Read this notice before unattended `/S` installation too; silent mode has no notice page.

If startup fails, use the bundled page's specific cause and next action. A missing key or unverified package cannot be repaired by repeated retries or an arbitrary older installer. For verified complete recovery, first preserve newer data and its matching program separately. Follow the exact batch's `RECOVERY.md` or Chinese guide and `PROGRAM-LOCATIONS.md`: restore the entire old data group, Desktop/uninstaller, standalone programs, command entries and native files, then the recorded current-user Apps registration and Task. Use the same ordinary account without elevation. Confirm Control/Gateway readiness, old configuration/routing/history and a real request before treating service as restored.

## Uninstall

Use **Installed apps → HiRoute → Uninstall**. Before removing the service, HiRoute restores its supported Agent configuration. If restoration needs a decision, uninstall stops and reports the Agent IDs; resolve them in HiRoute and retry. It removes only its recorded application files, Task, shell login entry and any CLI PATH entry it added. Data is retained unless you explicitly select its removal. The separate headless state directory is never used as the NSIS install directory.

If the service is unavailable, interactive recovery or uninstall offers a separate confirmation with **No** selected by default. Continuing may interrupt active work and leave configured Agents needing manual restoration; all HiRoute data is kept. Unattended mode refuses this exception, and a known restoration conflict still blocks removal. A failed first install with no Agent data can roll back without starting the service.

After an upgrade commits, obsolete verified package files may be removed. Material still required by storage recovery remains protected. Uninstall removes the current verified package too; changed or foreign files are preserved.
