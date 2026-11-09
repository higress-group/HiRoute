#!/usr/bin/env bash
set -euo pipefail

phase=$1
package=$2
revision=$3
name=ai.hiroute.desktop
unit=ai.hiroute.cli.service
marker="$HOME/.local/share/hiroute/standalone.json"
export XDG_RUNTIME_DIR="/run/user/$(id -u)"

root() {
    printf '%s\n' omarchy | sudo -S -p '' "$@"
}

find_wayland() {
    for _ in $(seq 1 80); do
        instance=$(hyprctl -j instances 2>/dev/null | jq -r '.[0].instance // empty') || instance=""
        if [[ -n "$instance" ]]; then
            export HYPRLAND_INSTANCE_SIGNATURE="$instance"
            return
        fi
        sleep 1
    done
    return 1
}

active_native_window() {
    hyprctl -j activewindow | jq -e --arg class "$name" '.class==$class and .xwayland==false'
}

if [[ "$phase" == prepare ]]; then
    [[ "$(uname -m)" == x86_64 ]]
    pacman -Q omarchy | grep -qE '^omarchy 4\.5\.'
    test ! -e "$marker"
    if pacman -Q hiroute-desktop >/dev/null 2>&1; then exit 1; fi
    cd "$HOME/release-check"
    sha256sum --check "$package.sha256"
    fingerprint=CB5F8ADEDC272ED8879D206C5642FA57EA8DF169
    gnupg=$(mktemp -d)
    [[ "$(gpg --homedir "$gnupg" --batch --show-keys --with-colons omarchy-release-key.asc |
        awk -F: '$1=="fpr" && !found { print $10; found=1 }')" == "$fingerprint" ]]
    root pacman-key --add omarchy-release-key.asc
    root pacman-key --lsign-key "$fingerprint"
    sed -E '/^[[:space:]#]*LocalFileSigLevel[[:space:]]*=/d; /^\[options\]$/a LocalFileSigLevel = Required TrustedOnly' \
        /etc/pacman.conf > "$PWD/pacman-strict.conf"
    root pacman --config "$PWD/pacman-strict.conf" --noconfirm -U "$PWD/$package"
    pacman -Q omarchy hyprland gtk3 webkit2gtk-4.1 hiroute-desktop
    test ! -e "$marker"
    hiroute-desktop --version | jq -e --arg revision "$revision" \
        'select(.source_revision==$revision and .arch=="x86_64" and .version=="0.6.0")'
    find_wayland
    hyprctl -i "$HYPRLAND_INSTANCE_SIGNATURE" dispatch 'hl.dsp.exec_cmd([[uwsm-app -- ai.hiroute.desktop.desktop]])' | grep -qx ok
    for _ in $(seq 1 60); do
        if active_native_window >/dev/null; then sleep 4; break; fi
        sleep 1
    done
    active_native_window
    test ! -e "$marker"
    exit 0
fi

[[ "$phase" == readiness ]]
cli="$HOME/.local/bin/hiroute"
for _ in $(seq 1 90); do
    if [[ -f "$marker" && -x "$cli" ]] && "$cli" system status --output json > /tmp/hiroute-kvm-status.json 2>/dev/null &&
       jq -e '.status=="succeeded" and .data.gateway=="ready" and .data.local_control=="v2.0:authenticated"' /tmp/hiroute-kvm-status.json >/dev/null; then
        break
    fi
    sleep 1
done
jq -e 'select(.status=="succeeded" and .data.gateway=="ready" and .data.local_control=="v2.0:authenticated") |
       {status,gateway:.data.gateway,local_control:.data.local_control}' /tmp/hiroute-kvm-status.json
"$cli" version --output json | jq -e --arg revision "$revision" \
    'select(.data.source_revision==$revision and .data.version=="0.6.0") | .data'
[[ "$(systemctl --user show "$unit" -p UnitFileState --value)" == disabled ]]
daemon_pid=$(systemctl --user show "$unit" -p MainPID --value)
[[ "$daemon_pid" =~ ^[1-9][0-9]*$ ]]
install_root=$(jq -er .install_root "$marker")
[[ "$(readlink "/proc/$daemon_pid/exe")" == "$install_root/hirouted" ]]
manifest=/usr/share/hiroute-desktop/standalone/manifest.json
for binary in hiroute hirouted; do
    expected=$(jq -er --arg entry "bin/$binary" '.files[$entry].sha256' "$manifest")
    printf '%s  %s\n' "$expected" "$install_root/$binary" | sha256sum --check
done
find_wayland
active_native_window
root pacman --noconfirm -R hiroute-desktop
[[ "$(systemctl --user show "$unit" -p MainPID --value)" == "$daemon_pid" ]]
"$cli" system status --output json | jq -e 'select(.status=="succeeded" and .data.gateway=="ready") |
    {status,package_removed:true,service_preserved:true}'
