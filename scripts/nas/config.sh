#!/usr/bin/env bash
# Export and import the full HiRoute configuration/state for this container.
#
# The archive is a consistent snapshot taken while the container is stopped:
#   data/state/hiroute/...   storage (control/runtime/secrets), console, gateway
#   data/share/hiroute/...   install marker and service data
# Credentials and history are included; treat the archive as a secret.
#
# Usage:
#   ./config.sh export [ARCHIVE]     # default: backups/hiroute-config-<version>-<stamp>.tar.gz
#   ./config.sh import ARCHIVE       # restores data/ after keeping a timestamped backup
#   ./config.sh verify ARCHIVE       # check digest and archive integrity
set -euo pipefail
cd "$(dirname "$0")"

COMPOSE=${COMPOSE:-docker compose}
SERVICE=hiroute
DATA_DIR=./data
BACKUP_DIR=./backups

die() { echo "error: $*" >&2; exit 1; }

service_running() {
    [ -n "$($COMPOSE ps -q "$SERVICE" 2>/dev/null)" ]
}

stop_service() {
    if service_running; then
        echo "Stopping $SERVICE for a consistent snapshot..."
        $COMPOSE stop "$SERVICE" >/dev/null
        STOPPED=1
    fi
}

start_service() {
    if [ "${STOPPED:-0}" = 1 ]; then
        echo "Starting $SERVICE..."
        $COMPOSE up -d "$SERVICE" >/dev/null
        STOPPED=0
    fi
}

read_env() {
    [ -f .env ] || die ".env not found; run cp env.example .env first"
    grep -E "^$1=" .env | tail -n1 | cut -d= -f2-
}

archive_manifest() {
    local archive="$1" created="$2" version="$3" target="$4"
    cat > "$archive.json" <<EOF
{
  "schema": "hiroute.nas-config-archive/v1",
  "created": "$created",
  "version": "$version",
  "target": "$target",
  "archive": "$(basename "$archive")",
  "contents": ["data/state/hiroute", "data/share/hiroute"],
  "excludes": ["data/state/hiroute/run", "data/state/hiroute/diagnostics"]
}
EOF
}

cmd_export() {
    local out="${1:-}"
    local version target created stamp
    version="$(read_env HIROUTE_VERSION)"
    target="$(uname -m)"
    case "$target" in
        x86_64|amd64) target=x86_64-unknown-linux-gnu ;;
        aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
        *) die "Unsupported architecture: $target" ;;
    esac
    created="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    stamp="$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$BACKUP_DIR"
    [ -n "$out" ] || out="$BACKUP_DIR/hiroute-config-${version}-${stamp}.tar.gz"
    [ -d "$DATA_DIR" ] || die "No data directory found; nothing to export"

    STOPPED=0
    trap start_service EXIT
    stop_service

    echo "Archiving $DATA_DIR (excluding runtime sockets and diagnostics)..."
    tar -czf "$out" \
        --exclude='data/state/hiroute/run' \
        --exclude='data/state/hiroute/diagnostics' \
        -C . data
    (cd "$(dirname "$out")" && sha256sum "$(basename "$out")" > "$(basename "$out").sha256")
    archive_manifest "$out" "$created" "$version" "$target"
    echo "Exported: $out"
    echo "Checksum: $out.sha256"
    echo "Manifest: $out.json"
}

cmd_verify() {
    local archive="$1"
    [ -f "$archive" ] || die "Archive not found: $archive"
    if [ -f "$archive.sha256" ]; then
        (cd "$(dirname "$archive")" && sha256sum -c "$(basename "$archive").sha256")
    else
        echo "warning: no .sha256 sidecar; skipping digest check" >&2
    fi
    tar -tzf "$archive" >/dev/null
    echo "Archive is readable: $archive"
    if [ -f "$archive.json" ]; then
        cat "$archive.json"
    fi
}

cmd_import() {
    local archive="$1"
    [ -f "$archive" ] || die "Archive not found: $archive"
    cmd_verify "$archive"

    local target
    if [ -f "$archive.json" ]; then
        target="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('target',''))" "$archive.json" 2>/dev/null || true)"
        case "$(uname -m)" in
            x86_64|amd64) host=x86_64-unknown-linux-gnu ;;
            aarch64|arm64) host=aarch64-unknown-linux-gnu ;;
            *) host="$(uname -m)" ;;
        esac
        if [ -n "$target" ] && [ "$target" != "$host" ]; then
            die "Archive target $target does not match this host $host"
        fi
    fi

    STOPPED=0
    trap start_service EXIT
    stop_service

    if [ -d "$DATA_DIR" ] && [ -n "$(ls -A "$DATA_DIR" 2>/dev/null)" ]; then
        local backup="${DATA_DIR}.backup.$(date +%Y%m%d-%H%M%S)"
        echo "Moving current data to $backup"
        mv "$DATA_DIR" "$backup"
    fi
    mkdir -p "$DATA_DIR"
    echo "Restoring $archive..."
    tar -xzf "$archive" -C .
    start_service
    trap - EXIT
    echo "Checking service..."
    $COMPOSE exec -T "$SERVICE" hiroute system status --output json
}

case "${1:-}" in
    export) shift; cmd_export "$@" ;;
    import) shift; [ $# -ge 1 ] || die "usage: ./config.sh import ARCHIVE"; cmd_import "$@" ;;
    verify) shift; [ $# -ge 1 ] || die "usage: ./config.sh verify ARCHIVE"; cmd_verify "$@" ;;
    *) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//' ;;
esac