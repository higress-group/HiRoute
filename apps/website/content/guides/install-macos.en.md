# Install and open HiRoute on macOS

HiRoute Desktop currently supports macOS 15.0 or later. Near-term releases are self-signed: they do not carry an Apple Developer ID signature or notarization, so macOS asks you to explicitly approve the first launch.

<details class="cli-reference" id="agent-cli">
<summary>Use an Agent: CLI map for this page</summary>

After installation, [enable the HiRoute management Skill in the product](/en/docs/#management-skill). Letting an Agent configure models and routes through the CLI is the recommended way to get started faster. The UI and CLI share configuration and records.

| Operation on this page | CLI hints (prefix each command with `hiroute`) |
| --- | --- |
| Check the CLI and service | `--help` · `system status` |
| Inspect connections and next steps | `quickstart` |
| Enable the built-in management Skill | `agents connect preview --management enable` → `agents connect apply` |
| Restore Agent connections before uninstalling | `agents restore preview` → `agents restore apply` |

These are command pointers. The Agent can read the installed version’s `--help` and `hiroute schema show --command-id COMMAND_ID --include-schema` for exact parameters.

</details>

## Download the right installer

Choose a DMG for your Mac on the [download page](/en/download/):

- Choose `arm64` for Apple silicon (M1, M2, M3, M4, and later).
- Choose `x86_64` for an Intel Mac.

Download from `hiroute.ai/releases/<version>/` and verify the file against the full SHA-256 on the download page. New releases use the short filename `HiRoute-<version>-macos-<arch>.dmg`:

```sh
shasum -a 256 ~/Downloads/HiRoute-*.dmg
```

Do not open a file whose digest does not match. Delete it and download it again.

## Install

1. Open the DMG.
2. Drag HiRoute into Applications.
3. Open HiRoute from Applications instead of continuing to run it from the DMG.

## Open a self-signed build for the first time

Try opening HiRoute normally first. If macOS blocks it:

1. Open System Settings → Privacy & Security.
2. Find the blocked HiRoute launch and choose Open Anyway.
3. Confirm that you want to open it.

Apple maintains the current steps in [Safely open apps on your Mac](https://support.apple.com/en-gb/102445). HiRoute does not ask you to disable Gatekeeper, weaken system security, or disable SIP.

If macOS says the file is damaged, verify the download domain and SHA256 and download it again. Do not treat an integrity failure as an ordinary first-launch warning.

## Install the Terminal entry (recommended)

After opening Desktop, go to Settings → CLI → Terminal entry and select Install. It installs the entry paired with the current app at:

```text
$HOME/.local/bin/hiroute
```

If Settings says that directory is not on PATH, add this line to `~/.zprofile`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Open a new Terminal window, then run:

```sh
hiroute --help
```

The Terminal entry connects to HiRoute's local service. Keep Desktop installed and the local service available. See [HiRoute CLI](/en/docs/cli/) for the released commands.

## Update and recover

Open **Settings → Updates** and select **Check for updates**. HiRoute gets stable releases from the official website, verifies the complete DMG and App, and checks upgrade prerequisites. Installation waits for current calls and tasks, and you can select **Cancel update** while it waits. Agent routing pauses during installation. HiRoute keeps the complete previous App and attempts to restore it if the update fails.

When an update changes the storage format, it retains configuration, credentials and routing and seals old HiRoute history. Current history and statistics begin at cutover; native Agent history is unchanged.

Prefer **Updates** over manual replacement. If you must replace the App manually, fully quit HiRoute, confirm that its service and tasks have stopped, and keep the complete original App and data directory.

After updating, check the service status on Home, reopen a saved route and send a small request from its connected Agent. This verifies active routing as well as saved configuration.

If startup fails, the startup page shows the specific cause and recovery instructions, even while the service is down. Read the `RECOVERY.md` of the batch the page identifies before restoring. You can also run `hiroute service recovery --output json` in Terminal to see the verified recovery batch and source directory. Do not replace individual database files, delete data, or retry repeatedly to work around a failure.

## Uninstall

1. On the **Agent** page, disable model routing, task routing and HiRoute management for each Agent, and wait for restoration to finish so each Agent returns to its earlier settings.
2. Turn off **Start at login** in **Settings**.
3. Fully quit HiRoute, then move the App to the Trash.

Deleting the App keeps your data by default. Only after confirming that no task is running, integrations have been restored, and session records and logs are no longer needed, delete the current user's HiRoute data under `~/Library/Application Support/ai.hiroute.desktop`. Never delete `.codex`, `.claude`, your projects or existing credentials.
