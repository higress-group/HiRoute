# Install HiRoute Desktop on Windows

HiRoute Desktop for Windows uses a current-user NSIS installer for native Windows x64. It includes the command line, background service, console files and a WebView2 bootstrapper. Python and administrator privileges are not required. Choose a Windows Desktop installer from the [download page](/en/download/).

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for your Agent">
<div class="agent-guide-heading"><strong>Let your Agent use this guide</strong></div>

Copy these instructions into your Agent conversation and describe what you want to do. They include documentation links, built-in Skill setup and CLI pointers.

<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read the documentation below, then use the HiRoute CLI to carry out my goal for “Install HiRoute Desktop on Windows.” Use the goal from our conversation and ask only for missing information you need.

Page documentation: https://hiroute.ai/en/docs/install-windows/
CLI reference: https://hiroute.ai/en/docs/cli/

Follow the guide for the requested installation or checks. Once installed, help me enable the built-in management Skill under “Agent → HiRoute management → Enable” for further configuration: https://hiroute.ai/en/docs/#management-skill

Command pointers (choose only what the goal needs; prefix each with hiroute):
- Check the CLI and service: --help · system status
- Inspect connections and next steps: quickstart
- Enable the built-in management Skill: agents connect preview --management enable → agents connect apply
- Restore Agent connections before uninstalling: agents restore preview → agents restore apply

The UI and CLI connect to the same HiRoute service and share configuration and execution records. Inspect and reuse existing state. Get exact parameters from the installed command’s --help and hiroute schema show --command-id COMMAND_ID --include-schema. Follow the built-in Skill for the required preview, apply and verification, then report the result.
```

</details>
</aside>

## Verify the download

Download from `hiroute.ai/releases/<version>/` and compare the full SHA-256 shown beside the file. The short filename for new releases is `HiRoute-<version>.exe`:

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath "$env:USERPROFILE\Downloads\YOUR-INSTALLER.exe"
```

[Get-FileHash](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-filehash) calculates the file digest. If it differs, delete the file and download it again.

## Open an unsigned installer

The installer is not code-signed. Windows may show an unknown publisher or a SmartScreen reputation warning. After verifying the source and digest, select **More info → Run anyway** when that choice is offered. An organization policy or Smart App Control may block this choice; use your organization's approval process. Do not disable Windows security. Microsoft describes these protections in [App & browser control](https://support.microsoft.com/en-us/windows/security/windows-security/app-browser-control-in-the-windows-security-app).

## Install and start

1. Open the installer in your ordinary account. It installs in `%LOCALAPPDATA%\Programs\HiRoute` and keeps service data separately under `%LOCALAPPDATA%\HiRoute\cli`.
2. Login startup and the tray are enabled by default. You can deselect either option. With the tray disabled, closing the window exits the shell; the background service continues.
3. Open HiRoute. It checks the current-user Task, starts the service, waits for the Gateway and opens its local console. There is no password or unlock step. Protected actions retain their page confirmation.

An existing current-user Task installation is upgraded. SCM (Windows service) installation is not offered. An existing SCM-managed installation is refused unchanged before product writes. ARM64 and x86 hosts are unsupported by this Desktop installer.

The installer adds the fixed `.local\bin` directory to your user PATH without replacing other entries. Open a new terminal for the updated PATH. Settings and the tray's **Start at login** action manage the shell and Task together. **Quit** exits the shell and leaves the service running.

For unattended installation, run the verified installer with `/S` and wait for its process exit code. It applies the default login-start and tray choices without prompts, starts the service and returns a nonzero code on failure. CLI-only users install this same package. There is no separate Windows standalone archive and no ARM64 package.

## Update and recover

Open **Settings → Updates**. The shell downloads a newer stable installer from the official HTTPS catalog and verifies its size and SHA-256 while your service keeps running. Installation waits for active calls and Worker tasks. **Cancel update** resumes normal admission without cancelling or replaying your work. The installer then replaces the application and reopens HiRoute.

A manually downloaded newer installer uses the same drain and transaction path. An older daemon that cannot drain requires an explicit interactive confirmation before it can be stopped; a passive update refuses it. If an update fails, the startup page or Updates shows the failure and recovery status. Keep the logs and retained backups. Starting the shell after an interrupted installer attempts journal recovery and resumes paused admission; changed or foreign files require repair and are preserved.

Copy the available failure diagnostics before retrying. When asking for help, include the failed stage, error codes and any recovery material identified by the page.

Complete-group recovery of data, matching programs and native state remains available, but compatibility with unpublished development builds is not guaranteed. Installation repair does not export Task/Apps registration or duplicate the complete Desktop unnecessarily.

A Core storage upgrade preserves configuration, credentials, routing and compatible HiRoute history. An Observation format change can retire older HiRoute history and start a new history generation. The recovery guide for the detected batch identifies the data and matching package needed for restoration. Review these steps before unattended `/S` installation.

If startup fails, use the bundled page's specific cause and next action. A missing key or unverified package cannot be repaired by repeated retries or an arbitrary older installer. For verified complete recovery, first preserve newer data and its matching program separately. Follow the exact batch's `RECOVERY.md` or Chinese guide and `PROGRAM-LOCATIONS.md`: restore the entire old data group, Desktop/uninstaller, standalone programs, command entries and native files, then the recorded current-user Apps registration and Task. Use the same ordinary account without elevation. Confirm Control/Gateway readiness, old configuration/routing/history and a real request before treating service as restored.

## Uninstall

Use **Installed apps → HiRoute → Uninstall**. Before removing the service, HiRoute restores its supported Agent configuration. If restoration needs a decision, uninstall stops and reports the Agent IDs; resolve them in HiRoute and retry. It removes only its recorded application files, Task, shell login entry and any CLI PATH entry it added. Data is retained unless you explicitly select its removal. The separate headless state directory is never used as the NSIS install directory.

If the service is unavailable, interactive recovery or uninstall offers a separate confirmation with **No** selected by default. Continuing may interrupt active work and leave configured Agents needing manual restoration; all HiRoute data is kept. Unattended mode refuses this exception, and a known restoration conflict still blocks removal. A failed first install with no Agent data can roll back without starting the service.

After an upgrade commits, obsolete verified package files may be removed. Material still required by storage recovery remains protected. Uninstall removes the current verified package too; changed or foreign files are preserved.
