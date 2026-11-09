#!/usr/bin/env bash
set -euo pipefail

revision=9fed4150abd9a331452d4bc1136d60a76d5ccb62
api=repos/higress-group/HiRoute/releases/438384449
stem=HiRoute-0.6.0-9fed4150abd9-omarchy-x86_64
upstream=https://nightly.omarchy.org/stable/4.5.0/479
work=$(mktemp -d)
evidence="$RUNNER_TEMP/omarchy-evidence"
mkdir -p "$evidence"
runner=""
trap 'if [[ -n "$runner" ]]; then kill "$runner" 2>/dev/null || true; wait "$runner" 2>/dev/null || true; fi' EXIT
draft_identity=$(gh api "$api" --jq '[.draft,.tag_name,.target_commitish]|@tsv')
[[ "$draft_identity" == $'true\tv0.6.0\t'"$revision" ]]

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

port=22555
env -u GH_TOKEN SSH_PORT="$port" MEM=5G bash "$work/vm-boot" "$work/omarchy.qcow2" \
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
ssh -p "$port" "${ssh_args[@]}" "$guest" 'uname -sm; pacman -Q omarchy; mkdir -m 700 "$HOME/release-check"' |
    tee "$evidence/guest-platform.txt"
python3 scripts/omarchy-qmp-check.py "$qmp" kvm | tee "$evidence/kvm.txt"

fetch "https://raw.githubusercontent.com/higress-group/HiRoute/$revision/assets/packaging/omarchy-release-key.asc" \
    "$work/omarchy-release-key.asc"

names=()
for _ in $(seq 1 100); do
    if assets=$(gh api "$api/assets" --paginate --jq '.[] | select(.state=="uploaded") | .name'); then
        for format in pkg.tar.xz pkg.tar.zst; do
            package="$stem.$format"
            ready=true
            for file in "$package" "$package.sig" "$package.sha256"; do
                if ! grep -qxF "$file" <<< "$assets"; then ready=false; fi
            done
            if [[ "$ready" == true ]]; then names=("$package" "$package.sig" "$package.sha256"); break; fi
        done
    fi
    [[ "${#names[@]}" == 3 ]] && break
    kill -0 "$runner"
    sleep 20
done
[[ "${#names[@]}" == 3 ]]
mkdir "$work/assets"
gh release download v0.6.0 -R higress-group/HiRoute --dir "$work/assets" --pattern "$package*"
(
    cd "$work/assets"
    sha256sum --check "$package.sha256"
    sha256sum "$package" "$package.sig" "$package.sha256"
) | tee "$evidence/package-checksums.txt"

scp -P "$port" "${ssh_args[@]}" "$work/assets/"* "$work/omarchy-release-key.asc" "$guest:release-check/"
ssh -p "$port" "${ssh_args[@]}" "$guest" bash -s -- prepare "$package" "$revision" \
    < scripts/validate-omarchy-kvm-guest.sh | tee "$evidence/guest-install.txt"
python3 scripts/omarchy-qmp-check.py "$qmp" screenshot "$work/setup.ppm"
tesseract "$work/setup.ppm" stdout 2>/dev/null | tee "$evidence/setup-ui.txt"
tr '\n' ' ' < "$evidence/setup-ui.txt" | grep -qi 'Install background service'
python3 scripts/omarchy-qmp-check.py "$qmp" install
ssh -p "$port" "${ssh_args[@]}" "$guest" bash -s -- readiness "$package" "$revision" \
    < scripts/validate-omarchy-kvm-guest.sh | tee "$evidence/guest-readiness.txt"
