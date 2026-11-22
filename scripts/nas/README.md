# HiRoute under Docker Compose (NAS and headless hosts)

Official HiRoute releases ship macOS and Windows desktop installers plus a Linux
standalone archive (`hiroute` + `hirouted` + `cliproxyapi`). No container image is
published, so this folder wraps the **official standalone archive** in a small
image and runs the production `hirouted --role all --standalone` daemon. All state
lives in two bind-mounted directories, so rebuilding the image never touches
configuration, credentials, routing plans or history.

The recipe is written for NAS hosts, but it works on any Docker host that can run
one container and keep a directory of persistent state.

## Layout

```
scripts/nas/
  Dockerfile            # debian:bookworm-slim + verified standalone archive
  docker-compose.yml    # single service, console port, state/share volumes
  entrypoint.sh         # first-boot records + console init, then uid 1000
  update.sh             # fetch and verify the latest stable package, refresh .env
  config.sh             # export/import a consistent configuration snapshot
  env.example           # copy to .env and edit
  backups/              # archives created by config.sh export
  data/                 # created on first boot; back this up
    state/              # /var/lib/hiroute/state/hiroute  (storage, console, gateway)
    share/              # /var/lib/hiroute/share/hiroute (standalone.json, service data)
```

Inside the image the archive is rearranged to match the official installer layout:
`/opt/hiroute/{hiroute,hirouted,console,libexec/cliproxyapi,...}`. The entrypoint
recreates the installer's `standalone.json` record in the data volume and refreshes
its version and CPA digest on upgrades, so the daemon's install-record identity
check keeps matching the image.

## Prerequisites

1. Docker Engine with the Compose plugin (`docker compose version` works).
2. A host architecture with a published standalone package: `x86_64` or `aarch64`.
   Check with `uname -m`.
3. Copy this folder to a persistent host directory, for example `/srv/hiroute`.
   NAS owners usually pick a shared folder such as `/volume1/docker/hiroute`.

## First deployment

```sh
cd /srv/hiroute
cp env.example .env
# edit ADVERTISE_ADDRESS (host LAN address) and CONSOLE_PORT if needed

# Downloads the current stable archive for this architecture, verifies its
# SHA-256 and writes ARCHIVE/ARCHIVE_SHA256/CPA_SHA256 into .env.
bash update.sh

docker compose build
docker compose up -d

# Print the one-time console login URL (expires in 2 minutes):
docker exec -it hiroute hiroute console open
```

Open the printed URL from a computer that can reach the host. The console listens
on `0.0.0.0:5838`; the URL advertises the address set in `.env`.

Useful checks:

```sh
docker exec -it hiroute hiroute system status --output json
docker exec -it hiroute hiroute console status
docker exec -it hiroute hiroute gateway show --output json
docker logs -f hiroute
```

If `console open` cannot reach the daemon socket, retry as the service user:
`docker exec -it -u 1000 hiroute hiroute console open`.

## Data and backup

- Persistent data is only `./data/state` and `./data/share`. Back up the whole
  `data/` directory before upgrading.
- `./data/state/hiroute/console-listener.json` holds the console settings and
  survives image rebuilds; the entrypoint only initializes it when missing.
- `./data/share/hiroute/standalone.json` is the install record. Keep it; it is
  refreshed in place on upgrades.
- The container may run as root; the entrypoint prepares the private directories
  and starts the daemon as uid 1000 (`hiroute`).

## Configuration export and import

Use `config.sh` to snapshot the full local state while the container is stopped
(clean SQLite state), with a checksum and a manifest, then restart the service:

```sh
bash config.sh export                       # backups/hiroute-config-<version>-<stamp>.tar.gz
bash config.sh export /srv/backups/hiroute-$(date +%F).tar.gz
bash config.sh verify backups/hiroute-config-0.6.4-20261122-120000.tar.gz
```

Restore on the same or another host of the same architecture:

```sh
bash config.sh import backups/hiroute-config-0.6.4-20261122-120000.tar.gz
```

- The archive contains `data/state/hiroute` (`storage/live/control.db`,
  `runtime.db`, `secrets.db`, `observation/activity.db`, keys) and
  `data/share/hiroute` (install marker). It includes API credentials and history,
  so treat it as a secret.
- `run/` sockets and `diagnostics/` logs are excluded.
- Import stops the service, moves the current `data/` to
  `data.backup.<timestamp>`, extracts the archive, then runs `system status`.
- If the shell user cannot read the uid-1000 private directories, run the command
  with `sudo`.
- Scheduled example (weekly, kept manually): add a cron entry such as
  `0 3 * * 0 /srv/hiroute/config.sh export`.

## Updating the image

`update.sh` reads the official `hiroute.ai/install.sh` to discover the newest
stable version, downloads the matching archive, verifies the manifest digest,
refreshes `.env`, and leaves the rebuild to you:

```sh
cd /srv/hiroute
bash update.sh
docker compose build
docker compose up -d
docker image prune -f        # optional: drop the previous image
```

Rollback before pruning if the new release misbehaves:

```sh
docker tag hiroute:<new-version> hiroute:backup
# restore the previous HIROUTE_VERSION/ARCHIVE/ARCHIVE_SHA256/CPA_SHA256 in .env
docker compose build && docker compose up -d
```

Manual equivalent: download `HiRoute-<version>-linux-<arch>.tar.gz` and its `.json`
manifest from <https://hiroute.ai/en/download/> into this folder, update
`HIROUTE_VERSION`, `ARCHIVE_SHA256` and `CPA_SHA256` in `.env`, then rebuild. The
daemon migrates local storage on first start of a newer release; console settings,
tokens and data are preserved.

## Notes and troubleshooting

- **Port conflict:** change `CONSOLE_PORT` in `.env` before first boot, then
  rebuild. An occupied explicit port fails instead of picking another one.
- **Console unreachable:** verify `ADVERTISE_ADDRESS` is an address the browser can
  reach, the port is open in the host firewall, and
  `docker exec -it hiroute hiroute console status` shows an applied listener.
- **Remote Agents:** the Gateway defaults to loopback. If Codex or Claude Code runs
  on another machine and must reach the Gateway, configure the Gateway listener in
  the console or CLI and add the corresponding port to `docker-compose.yml`; keep
  it inside a trusted network or a VPN.
- **Architecture:** `update.sh` auto-detects `uname -m`; override with
  `HIROUTE_ARCH=x86_64 bash update.sh`. Cross-building on another host requires a
  `platform:` entry in `docker-compose.yml` and is not covered here.
- **No official image:** this is a wrapper around the official standalone archive,
  not an official HiRoute image. The archive digest is pinned in `.env`; review
  `update.sh` output before rebuilding.
