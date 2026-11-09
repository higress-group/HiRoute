#!/usr/bin/env bash
set -euo pipefail

signed=$(realpath "${1:?signed packages required}")
previous=$(realpath "${2:?recorded predecessor required}")
revision=$(jq -er .revision "$signed/signed.json")
version=$(jq -er .version "$signed/signed.json")
predecessor=$(jq -er .revision "$previous/predecessor.json")
prior_version=$(jq -er .version "$previous/predecessor.json")
archive=$(jq -er .archive "$previous/predecessor.json")
package=$(jq -er '.packages[] | select(.architecture=="x86_64") | .filename' "$signed/signed.json")
package_digest=$(jq -er '.packages[] | select(.architecture=="x86_64") | .sha256' "$signed/signed.json")
[[ "$revision" =~ ^[0-9a-f]{40}$ && "$predecessor" =~ ^[0-9a-f]{40}$ ]]
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$prior_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$archive" == "hiroute-$prior_version-${predecessor:0:12}-x86_64-unknown-linux-gnu.tar.gz" ]]
stem="HiRoute-$version-${revision:0:12}-omarchy-x86_64"
[[ "$package" == "$stem.pkg.tar.xz" || "$package" == "$stem.pkg.tar.zst" ]]
[[ "$package_digest" =~ ^[0-9a-f]{64}$ ]]
upstream=https://nightly.omarchy.org/stable/4.5.0/479
work=$(mktemp -d)
evidence="$RUNNER_TEMP/omarchy-evidence"
mkdir -p "$evidence"
runner=""
witness=""
cleanup() {
    for child in "$witness" "$runner"; do
        if [[ -n "$child" ]]; then
            kill "$child" 2>/dev/null || true
            wait "$child" 2>/dev/null || true
        fi
    done
}
trap cleanup EXIT
printf 'validation revision %s\npackage revision %s\npredecessor revision %s\n' \
    "$(git rev-parse HEAD)" "$revision" "$predecessor" > "$evidence/scope.txt"

fetch() {
    curl --fail --silent --show-error --location --retry 3 --proto '=https' "$1" -o "$2"
}

fetch "$upstream/omarchy-4.5.0.qcow2" "$work/omarchy.qcow2" &
image_fetch=$!
for file in vmlinuz initrd cmdline vm-boot; do fetch "$upstream/unpacked/$file" "$work/$file"; done
wait "$image_fetch"
(
    cd "$work"
    sha256sum --check <<'SUMS'
dcb401aabf1b01bf3c983a1b048bcd0ac7d66e4f5a755acd9452bf66e18cd5c4  omarchy.qcow2
b4aae0fbbcfe8893671f504b2971373c43350d029d28f1c9d69d8f516fae6756  vmlinuz
b38fca90c90f480811abcd6ccfe7113d08c6a161f6e434316d05278f47551379  initrd
7d758d7619cca809fcbca360b1e4d0a8da484271cbc19f39a81d44e954e39603  cmdline
fc2becf1d3f3441ab45e972ca4904b5808800e819a1a0308bc4bc74424388382  vm-boot
SUMS
) | tee "$evidence/upstream-checksums.txt"

git show "$revision:assets/packaging/omarchy-release-key.asc" > "$work/omarchy-release-key.asc"
(
    cd "$signed"
    printf '%s  %s\n' "$package_digest" "$package" | sha256sum --check
    sha256sum --check "$package.sha256"
    sha256sum "$package" "$package.sig" "$package.sha256"
) | tee "$evidence/package-checksums.txt"

port=22555
SSH_PORT="$port" MEM=5G \
    QEMU_EXTRA="-device qemu-xhci -device usb-tablet" bash "$work/vm-boot" "$work/omarchy.qcow2" \
    > "$work/vm-login.txt" 2> "$evidence/vm-boot.log" &
runner=$!
for _ in $(seq 1 300); do
    grep -q 'QMP socket:' "$work/vm-login.txt" && break
    kill -0 "$runner"
    sleep 1
done
key=$(awk '$1=="ssh" { print $3; exit }' "$work/vm-login.txt")
qmp=$(sed -n 's/^.*QMP socket: \(.*\)\. Ctrl-C.*$/\1/p' "$work/vm-login.txt")
test -f "$key"
test -S "$qmp"
ssh_args=(-i "$key" -o BatchMode=yes -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
guest=omarchy@127.0.0.1
guest_ssh=(ssh -T -p "$port" "${ssh_args[@]}" "$guest")
guest_copy=(scp -P "$port" "${ssh_args[@]}")
"${guest_ssh[@]}" 'uname -sm; pacman -Q omarchy; mkdir -m 700 "$HOME/release-check"' |
    tee "$evidence/guest-platform.txt"
python3 scripts/omarchy-qmp-check.py "$qmp" kvm | tee "$evidence/kvm.txt"

guest_phase() {
    "${guest_ssh[@]}" bash -s -- "$1" "$2" "$3" "$4" < scripts/validate-omarchy-kvm-guest.sh
}

wait_for_witness() {
    local case_name=$1 sequence=$2 deadline=$((SECONDS + 120))
    while ! jq -se --arg case "$case_name" --argjson sequence "$sequence" '
        any(.[]; .case==$case and (.sequence // 0)==$sequence and .result=="passed")
    ' "$evidence/plan-witness.jsonl" >/dev/null 2>&1; do
        kill -0 "$witness" 2>/dev/null && (( SECONDS < deadline )) || {
            printf 'Plan witness did not complete %s %s\n' "$case_name" "$sequence" >&2
            return 1
        }
        sleep 1
    done
}

"${guest_copy[@]}" "$previous/$archive" "$previous/$archive.json" "$previous/install-predecessor.py" "$guest:release-check/"
guest_phase predecessor "$archive" "$predecessor" "$prior_version" | tee "$evidence/predecessor.txt"
"${guest_copy[@]}" -r crates/daemon/tests/support scripts/omarchy-kvm-plan.py \
    crates/integrations/src/agents/codex_bundled_catalog.json "$guest:release-check/"
mkfifo "$work/witness-input"
exec 9<> "$work/witness-input"
"${guest_ssh[@]}" 'export XDG_RUNTIME_DIR="/run/user/$(id -u)"; cd "$HOME/release-check"; exec python3 -u omarchy-kvm-plan.py' <&9 9>&- \
    > "$evidence/plan-witness.jsonl" 2> "$work/witness-private.stderr" &
witness=$!
wait_for_witness seed 0
printf 'request\n' >&9
wait_for_witness gateway 1

"${guest_copy[@]}" "$signed/$package" "$signed/$package.sig" \
    "$signed/$package.sha256" "$work/omarchy-release-key.asc" "$guest:release-check/"
guest_phase prepare "$package" "$revision" "$version" | tee "$evidence/guest-install.txt"

record_ui() {
    local stage=$1
    guest_phase active "$package" "$revision" "$version" > "$evidence/$stage-window.json"
    tesseract "$work/$stage.ppm" stdout --psm 11 2>/dev/null > "$evidence/$stage-ui.txt"
}

coordinates=$(python3 scripts/omarchy-qmp-check.py "$qmp" locate "$work/preview.ppm" 60 \
    'Check for service update' 'Continue in your browser')
read -r x y width height <<< "$coordinates"
record_ui preview
printf 'Check for service update at %s\n' "$coordinates" > "$evidence/preview-action.txt"
python3 scripts/omarchy-qmp-check.py "$qmp" click "$x" "$y" "$width" "$height"

python3 scripts/omarchy-qmp-check.py "$qmp" await-screen "$work/confirm.ppm" 135 \
    'Update background service' "Update from $prior_version to $version"
record_ui confirm
# The startup view focuses Cancel when it renders this confirmation.
printf '%s\n' 'Tab from Cancel to Update and restart, then Space' > "$evidence/confirm-action.txt"
python3 scripts/omarchy-qmp-check.py "$qmp" confirm
guest_phase readiness "$package" "$revision" "$version" | tee "$evidence/guest-readiness.txt"

python3 scripts/omarchy-qmp-check.py "$qmp" await-screen "$work/console.ppm" 90 \
    'Home' 'Models|Recent work' 'Routing|things that need your attention' > "$evidence/console-navigation.txt"
printf 'request\n' >&9
wait_for_witness gateway 2
printf 'exit\n' >&9
exec 9>&-
wait "$witness"
witness=""
jq -se '
    map(select(.case=="gateway")) as $requests |
    map(select(.case=="seed"))[0].managed_codex_sha256 as $config |
    ($config | test("^[a-f0-9]{64}$")) and
    ($requests | map(.sequence)) == [1,2] and
    all($requests[]; .result=="passed" and .status==200 and .fixture_requests==1
        and .source_preserved and .plan_preserved and .observed
        and .managed_codex_sha256==$config) and
    ($requests[1].previous_receipt_preserved == true)
' "$evidence/plan-witness.jsonl" | tee "$evidence/plan-verified.txt"
