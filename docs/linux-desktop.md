# HiRoute on Omarchy

The Arch package supplies the HiRoute launcher, desktop executable and a verified
standalone payload for the same build. The desktop and CLI use the current user's
systemd service. Closing the window leaves the service running. Removing the
package removes its system files and launcher. User runtime removal is a separate
operation described in the [standalone CLI guide](standalone-cli.md).

## Install and open

Download the `hiroute-desktop` package, its detached signature and checksums for
your machine from the [HiRoute release](https://github.com/higress-group/HiRoute/releases).
Verify the signature against the release's independently published signing-key
fingerprint and check the package digest. Install the verified file with
`sudo pacman -U ./hiroute-desktop-<version>-1-<arch>.pkg.tar.zst`.

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
checksum. Release signing and publication happen in the release process.

For an interrupted user runtime install, use **Recover installation** on the
startup screen or the [standalone recovery command](standalone-cli.md). Keep the
current data, package and install record until recovery finishes.
