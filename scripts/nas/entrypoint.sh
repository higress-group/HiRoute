#!/bin/sh
set -eu

HOME=/var/lib/hiroute
export HOME
export XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/state}"
export XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/share}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/hiroute}"

# The container may run as root (common for NAS Docker setups); prepare private
# directories and hand the daemon to the unprivileged hiroute account.
install -d -m 0700 /run/hiroute
chmod 0700 /run/hiroute
chown -R hiroute:hiroute /run/hiroute
install -d "$XDG_STATE_HOME/hiroute" "$XDG_DATA_HOME/hiroute"
chown hiroute:hiroute "$XDG_STATE_HOME/hiroute" "$XDG_DATA_HOME/hiroute"
chmod 0700 "$XDG_STATE_HOME/hiroute" "$XDG_DATA_HOME/hiroute"

as_user() {
    setpriv --reuid=1000 --regid=1000 --clear-groups "$@"
}

marker="$XDG_DATA_HOME/hiroute/standalone.json"
case "$(uname -m)" in
    x86_64|amd64) target=x86_64-unknown-linux-gnu ;;
    aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
    *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
: "${HIROUTE_VERSION:?HIROUTE_VERSION is required}"
: "${CPA_SHA256:?CPA_SHA256 is required}"

# Recreate the installer's standalone.json record on first boot, or refresh its
# version/CPA digest after an image upgrade so the daemon identity check keeps
# matching this image.
if [ ! -f "$marker" ]; then
    cat > "$marker" <<EOF
{
  "schema_version": "hiroute.standalone-install/v1",
  "version": "$HIROUTE_VERSION",
  "target": "$target",
  "install_root": "/opt/hiroute",
  "service_definition": "/opt/hiroute/hiroute.service",
  "installed_skills": [],
  "cpa_binary": "/opt/hiroute/libexec/cliproxyapi",
  "cpa_sha256": "$CPA_SHA256"
}
EOF
else
    sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$HIROUTE_VERSION\"/" "$marker"
    sed -i "s/\"cpa_sha256\": \"[^\"]*\"/\"cpa_sha256\": \"$CPA_SHA256\"/" "$marker"
fi
chown hiroute:hiroute "$marker"
chmod 0600 "$marker"

# First boot only: persist the console listener settings. Existing settings in
# the state volume are preserved across image rebuilds and upgrades.
if [ ! -f "$XDG_STATE_HOME/hiroute/console-listener.json" ]; then
    case "${CONSOLE_MODE:-network}" in
        network)
            : "${ADVERTISE_ADDRESS:?ADVERTISE_ADDRESS must be set for network console mode}"
            as_user /opt/hiroute/hiroute console init \
                --mode network --listen 0.0.0.0 \
                --advertise-address "$ADVERTISE_ADDRESS" \
                --port "${CONSOLE_PORT:-5838}" --allow-plaintext-network
            ;;
        loopback)
            as_user /opt/hiroute/hiroute console init \
                --mode loopback --port "${CONSOLE_PORT:-5838}"
            ;;
        off)
            ;;
        *)
            echo "Unknown CONSOLE_MODE: $CONSOLE_MODE" >&2
            exit 1
            ;;
    esac
fi

exec setpriv --reuid=1000 --regid=1000 --clear-groups \
    /opt/hiroute/hirouted --role all --standalone \
    --cpa-binary /opt/hiroute/libexec/cliproxyapi \
    --cpa-sha256 "$CPA_SHA256"