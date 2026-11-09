# HiRoute on Omarchy

The Arch package supplies the HiRoute launcher, desktop executable and a verified
standalone payload for the same build. The desktop and CLI use the current user's
systemd service. Closing the window leaves the service running. Removing the
package removes its system files and launcher. User runtime removal is a separate
operation described in the [standalone CLI guide](standalone-cli.md).

## Install and open

Download the `hiroute-desktop` package, its detached signature and checksums for
your machine from the [HiRoute release](https://github.com/higress-group/HiRoute/releases).
Download the [HiRoute Omarchy release public key](../assets/packaging/omarchy-release-key.asc)
from the repository. Use `gpg --show-keys --with-fingerprint omarchy-release-key.asc`
to check its primary fingerprint against `CB5F8ADEDC272ED8879D206C5642FA57EA8DF169`.
Keep the package, its `.sig` and its `.sha256` together and check the digest.
Once you have verified the fingerprint, add and locally sign the release key
in the pacman keyring, then install the downloaded package.

```sh
package=HiRoute-0.6.0-REVISION-omarchy-ARCH.pkg.tar.xz
sha256sum --check "$package.sha256"
sudo pacman-key --add omarchy-release-key.asc
sudo pacman-key --lsign-key CB5F8ADEDC272ED8879D206C5642FA57EA8DF169
sudo pacman -U "$package"
```

Set `package` to the exact downloaded filename. Pacman verifies the adjacent
signature using its normal signature policy.

Open **HiRoute** from the app launcher. On first run, **Install background service**
sets up the current user's CLI, service and local console from the package. The
background service can also start at login if selected. An existing verified
installation keeps its listener configuration and service settings. If its
console is available but requires a different build, **Open in browser** opens a
separate authenticated local session. The startup view provides terminal steps
when the local console is disabled.

The Appearance preference defaults to **Follow system**. Light and Dark override
the system appearance for HiRoute and survive a restart. The window uses Wayland
framing and the standard Omarchy launcher identity. The product UI shares the
same controls and palette across Desktop and Web.

## Update or remove the background service

Install a newer Arch package with `pacman -U`. If the running service is an
eligible earlier release, select **Check for service update…** on the startup screen.
Review the versions and restart impact, then choose **Update and restart**.
Choose **Open in browser** to keep using the existing service until you are ready.
An update carries forward the existing listener, login-start setting and data.

To remove the current user's runtime, open **Settings**, find **Desktop app**
and choose **Remove service…**. Review the listed Agents before confirming.
HiRoute restores their eligible managed settings, then stops the service and
removes the owned CLI and user unit. Use **Agents** to resolve settings that
require attention before retrying. Saved sources, Plans and history are available
when you reinstall the runtime. The package and its launcher stay installed.

## Build a local candidate

Use a native Arch build host for the target architecture. Build a verified
standalone bundle from the same clean source revision with the
[release tooling](release-builds.md), then run:

```sh
python3 scripts/package-linux-desktop.py \
  --manifest path/to/standalone.tar.gz.json \
  --archive path/to/standalone.tar.gz \
  --output artifacts/linux-desktop
```

The build expects Rust, Node/npm, the GTK3 and WebKitGTK 4.1 development
interfaces, and Arch package tooling. It produces a candidate package and
checksum. The release process signs the final package and verifies that its
embedded runtime and manifest match the separately qualified standalone files.

For an interrupted user runtime install, use **Recover installation** on the
startup screen or the [standalone recovery command](standalone-cli.md). Keep the
current data, package and install record until recovery finishes.
