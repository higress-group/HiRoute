# Install and open HiRoute on Omarchy

HiRoute provides a signed Arch package for Omarchy on x86_64 and ARM64. It includes the desktop app, launcher and matching CLI/service payload. The app and CLI share the current user's models, routes, Agents and sessions.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for your Agent">
<div class="agent-guide-heading"><strong>Let your Agent use this guide</strong></div>

Copy these instructions into your Agent conversation and describe what you want to do. They include documentation links, built-in Skill setup and CLI pointers.

<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read the documentation below, then use the HiRoute CLI to carry out my goal for “Install and open HiRoute on Omarchy.” Use the goal from our conversation and ask only for missing information you need.

Page documentation: https://hiroute.ai/en/docs/install-omarchy/
CLI reference: https://hiroute.ai/en/docs/cli/

Follow the guide to choose the architecture, verify the OpenPGP fingerprint and package signature, and install with pacman. Open HiRoute from Apps and use its startup screen to install or update the user service. Once installed, help me enable the built-in management Skill under “Agent → HiRoute management → Enable” for further configuration: https://hiroute.ai/en/docs/#management-skill

Command pointers (choose only what the goal needs; prefix each with hiroute):
- Check the CLI and service: --help · system status
- Inspect connections and next steps: quickstart
- Enable the built-in management Skill: agents connect preview --management enable → agents connect apply
- Restore Agent connections before uninstalling: agents restore preview → agents restore apply

The UI and CLI connect to the same HiRoute service and share configuration and execution records. Inspect and reuse existing state. Get exact parameters from the installed command’s --help and hiroute schema show --command-id COMMAND_ID --include-schema. Follow the built-in Skill for the required preview, apply and verification, then report the result.
```

</details>
</aside>

## 1. Choose your architecture

Run `uname -m` in a terminal. Choose **x86_64** for `x86_64`, or **ARM64** for `aarch64`, on the [Omarchy installation page](/en/download/#omarchy).

Save the package, its `.sig` and its `.sha256` file in the same folder. The installation page gives commands for the exact selected package. Keep its original filename, including `.pkg.tar.xz` or `.pkg.tar.zst`.

## 2. Verify the package and release key

In the download folder, set `package` to the exact downloaded package filename, then check its checksum.

```sh
package='PASTE_EXACT_PACKAGE_FILENAME'
sha256sum --check "$package.sha256"
```

Download the [HiRoute release public key](https://raw.githubusercontent.com/higress-group/HiRoute/main/assets/packaging/omarchy-release-key.asc) as `omarchy-release-key.asc` and inspect it.

```sh
gpg --show-keys --with-fingerprint omarchy-release-key.asc
```

The primary fingerprint must match `CB5F8ADEDC272ED8879D206C5642FA57EA8DF169`. Stop if the fingerprint or checksum differs, and recheck the source and files.

## 3. Install the signed package

After checking the fingerprint, add the release key to pacman's keyring and locally trust that key. Install the package with its adjacent signature.

```sh
sudo pacman-key --add omarchy-release-key.asc &&
sudo pacman-key --lsign-key CB5F8ADEDC272ED8879D206C5642FA57EA8DF169 &&
sudo pacman-key --verify "$package.sig" "$package" &&
sudo pacman -U "$package"
```

Installation starts only after signature verification succeeds. If the `.sig` file is missing or verification fails, recheck the source and files.

## 4. Open HiRoute from Apps

Open **HiRoute** from the Omarchy app launcher. On first launch, choose **Install background service** to set up the current user's CLI, service and local console. You can also choose to start the service at login. An existing compatible installation retains its listener and service settings.

When the console opens, [connect your first model](/en/docs/models/), then follow the [Quickstart](/en/docs/). Closing the desktop window leaves the background service running.

## Update HiRoute

Download and verify the newer package, then install it with `pacman -U`. When the startup screen offers **Check for service update…**, review the versions and restart impact, then choose **Update and restart**. This preserves your existing configuration and data.

**Open in browser** lets you keep using an existing service while arranging its update. If the local console is disabled, follow the terminal steps on the startup screen.

## Remove the service or application

To remove the user runtime, open **Settings → Desktop app → Remove service…** and review the affected Agents. Resolve any Agent settings that need attention before retrying. Saved models, Plans and history remain available for reinstallation.

The system package and launcher are removed separately with `sudo pacman -R hiroute-desktop`. Removing the package alone leaves the current user's runtime and data in place.
