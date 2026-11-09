#!/usr/bin/env bash
set -euo pipefail
umask 077

phase=${1:?Missing guest phase}
artifact=${2:?Missing artifact name}
revision=${3:?Missing source revision}
version=${4:?Missing package version}
name=ai.hiroute.desktop
unit=ai.hiroute.cli.service
marker="$HOME/.local/share/hiroute/standalone.json"
cli="$HOME/.local/bin/hiroute"
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
cd "$HOME/release-check"

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
    hyprctl -j activewindow | jq -ce --arg class "$name" '
        select(.class==$class and .xwayland==false) | {class,pid,size,xwayland}'
}

daemon_pid() {
    systemctl --user show "$unit" -p MainPID --value
}

if [[ "$phase" == predecessor ]]; then
    [[ "$(uname -m)" == x86_64 ]]
    test ! -e "$marker"
    if pacman -Q hiroute-desktop >/dev/null 2>&1; then exit 1; fi
    python3 -I install-predecessor.py install --manifest "$artifact.json" --archive "$artifact" \
        --console-mode loopback --console-port 5841 >/dev/null
    "$cli" service start --output json >/dev/null
    "$cli" service status --output json | jq -e '.data | select(.local_control_ready and (.autostart_enabled|not)) |
        {local_control_ready,autostart_enabled}'
    "$cli" version --output json | jq -e --arg revision "$revision" --arg version "$version" \
        '.data | select(.source_revision==$revision and .arch=="x86_64" and .version==$version)'
    pid=$(daemon_pid)
    [[ "$pid" =~ ^[1-9][0-9]*$ ]]
    printf '%s\n' "$pid" > predecessor.pid
    printf 'Released service PID %s\n' "$pid"
    exit 0
fi

if [[ "$phase" == prepare ]]; then
    [[ "$(uname -m)" == x86_64 ]]
    pacman -Q omarchy | grep -qE '^omarchy 4\.5\.'
    if pacman -Q hiroute-desktop >/dev/null 2>&1; then exit 1; fi
    test -f "$marker"
    [[ "$(daemon_pid)" == "$(cat predecessor.pid)" ]]
    sha256sum --check "$artifact.sha256"
    fingerprint=CB5F8ADEDC272ED8879D206C5642FA57EA8DF169
    gnupg=$(mktemp -d)
    [[ "$(gpg --homedir "$gnupg" --batch --show-keys --with-colons omarchy-release-key.asc |
        awk -F: '$1=="fpr" && !found { print $10; found=1 }')" == "$fingerprint" ]]
    root pacman-key --add omarchy-release-key.asc
    root pacman-key --lsign-key "$fingerprint"
    sed -E '/^[[:space:]#]*LocalFileSigLevel[[:space:]]*=/d; /^\[options\]$/a LocalFileSigLevel = Required TrustedOnly' \
        /etc/pacman.conf > "$PWD/pacman-strict.conf"
    root pacman --config "$PWD/pacman-strict.conf" --noconfirm -U "$PWD/$artifact"
    pacman -Q omarchy hyprland gtk3 webkit2gtk-4.1 hiroute-desktop
    [[ "$(daemon_pid)" == "$(cat predecessor.pid)" ]]
    printf 'Package installation preserved released service PID %s\n' "$(daemon_pid)"
    hiroute-desktop --version | jq -e --arg revision "$revision" --arg version "$version" \
        'select(.source_revision==$revision and .arch=="x86_64" and .version==$version)'
    find_wayland
    hyprctl dispatch 'hl.dsp.exec_cmd([[uwsm-app -- ai.hiroute.desktop.desktop]])' | grep -qx ok
    for _ in $(seq 1 60); do
        if active_native_window >/dev/null; then break; fi
        sleep 1
    done
    window=$(active_native_window)
    printf '%s\n' "$window"
    jq -er .pid <<< "$window" > desktop.pid
    exit 0
fi

if [[ "$phase" == active ]]; then
    find_wayland
    window=$(active_native_window)
    [[ "$(jq -er .pid <<< "$window")" == "$(cat desktop.pid)" ]]
    [[ "$(daemon_pid)" == "$(cat predecessor.pid)" ]]
    printf '%s\n' "$window"
    exit 0
fi

[[ "$phase" == readiness ]]
ready=false
for _ in $(seq 1 240); do
    if [[ -f "$marker" && -x "$cli" ]] &&
       "$cli" version --output json 2>/dev/null | jq -e --arg revision "$revision" --arg version "$version" \
           '.data.source_revision==$revision' >/dev/null &&
       "$cli" system status --output json > status.json 2>/dev/null &&
       jq -e '.status=="succeeded" and .data.gateway=="ready" and .data.local_control=="v2.0:authenticated"' status.json >/dev/null; then
        ready=true
        break
    fi
    sleep 1
done
[[ "$ready" == true ]]
jq '{status,gateway:.data.gateway,local_control:.data.local_control}' status.json
"$cli" version --output json | jq -e --arg revision "$revision" --arg version "$version" \
    '.data | select(.source_revision==$revision and .arch=="x86_64" and .version==$version)'
[[ "$(systemctl --user show "$unit" -p UnitFileState --value)" == disabled ]]
pid=$(daemon_pid)
[[ "$pid" =~ ^[1-9][0-9]*$ ]]
[[ "$pid" != "$(cat predecessor.pid)" ]]
install_root=$(jq -er .install_root "$marker")
[[ "$(readlink "/proc/$pid/exe")" == "$install_root/hirouted" ]]
"/proc/$pid/exe" --version | jq -e --arg revision "$revision" --arg version "$version" \
    'select(.source_revision==$revision and .arch=="x86_64" and .version==$version)'
manifest=/usr/share/hiroute-desktop/standalone/manifest.json
for binary in hiroute hirouted; do
    expected=$(jq -er --arg entry "bin/$binary" '.files[$entry].sha256' "$manifest")
    printf '%s  %s\n' "$expected" "$install_root/$binary" | sha256sum --check
done
find_wayland
window=$(active_native_window)
[[ "$(jq -er .pid <<< "$window")" == "$(cat desktop.pid)" ]]
printf '%s\n' "$window"
