#!/usr/bin/env bash
# Run makepkg and the release-profile shell build on the native architecture.
set -euo pipefail
probe=${1:-}
control=$(cd "$(dirname "$0")/.." && pwd -P)
arch=$(uname -m)
if [[ "$probe" != --probe ]]; then
    source_dir=$(realpath "${1:?candidate checkout required}")
    inputs=$(realpath "${2:?qualified runtime directory required}")
    destination=${3:?package output directory required}
    version=$(jq -er '.version | select(test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))' "$inputs/candidate.json")
    revision=$(git -C "$source_dir" rev-parse HEAD)
    [[ "$revision" == "$(jq -er .revision "$inputs/candidate.json")" ]]
    archive="hiroute-$version-${revision:0:12}-$arch-unknown-linux-gnu.tar.gz"
    test -f "$inputs/$arch/$archive"
    test -f "$inputs/$arch/$archive.json"
fi

scratch=$(mktemp -d)
mkdir "$scratch/home" "$scratch/packages"
fetch() {
    curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 \
        --max-time 180 --proto '=https' "$1" --output "$2"
}
case "$arch" in
    x86_64)
        base=ghcr.io/archlinux/archlinux@sha256:8942cdde991720d8cc0674c0c7ea47b0b29c0f09ed2f83ade94356a106461157
        keyring=archlinux
        mirror='https://stable-mirror.omarchy.org/$repo/os/$arch'
        ;;
    aarch64)
        # Arch Linux ARM publishes a signed generic root filesystem. Pin its
        # distribution key and record the exact rootfs used by this producer.
        upstream=https://ca.us.mirror.archlinuxarm.org/os/ArchLinuxARM-aarch64-latest.tar.gz
        fingerprint=68B3537F39A313B3E574D06777193F152BDBE6A6
        key_home=$(mktemp -d)
        fetch "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x$fingerprint" "$scratch/arch-arm.asc"
        [[ "$(gpg --no-options --homedir "$key_home" --batch --show-keys --with-colons "$scratch/arch-arm.asc" |
            awk -F: '$1=="pub" { primary=1; next } $1=="fpr" && primary { print $10; primary=0 }')" == "$fingerprint" ]]
        gpg --no-options --homedir "$key_home" --batch --dearmor --output "$scratch/arch-arm.gpg" "$scratch/arch-arm.asc"
        fetch "$upstream" "$scratch/arch-arm.tar.gz"
        fetch "$upstream.sig" "$scratch/arch-arm.sig"
        gpgv --homedir "$key_home" --keyring "$scratch/arch-arm.gpg" "$scratch/arch-arm.sig" "$scratch/arch-arm.tar.gz"
        sha256sum "$scratch/arch-arm.tar.gz"
        base=hiroute-archlinuxarm:verified
        docker import --platform linux/arm64 "$scratch/arch-arm.tar.gz" "$base"
        unlink "$scratch/arch-arm.tar.gz"
        keyring=archlinuxarm
        mirror='https://arm-stable-mirror.omarchy.org/$arch/$repo'
        ;;
    *)
        echo "Unsupported native architecture $arch" >&2
        exit 1
        ;;
esac

image="hiroute-omarchy-builder:$arch"
docker build --file "$control/scripts/omarchy-builder.Dockerfile" --tag "$image" \
    --build-arg "BASE=$base" --build-arg "KEYRING=$keyring" --build-arg "MIRROR=$mirror" \
    --build-arg "BUILD_UID=$(id -u)" --build-arg "BUILD_GID=$(id -g)" "$scratch/home"
docker image inspect --format '{{.Id}}' "$image"

if [[ "$probe" == --probe ]]; then
    docker run --rm --user "$(id -u):$(id -g)" \
        --mount "type=bind,source=$control/apps/desktop/arch/PKGBUILD,target=/PKGBUILD,readonly" \
        "$image" bash -ec '
            source /PKGBUILD
            pacman -T "${depends[@]}"
            pacman -Q glibc gtk3 webkit2gtk-4.1
            makepkg --version
        '
    exit
fi

node_dir=$(dirname "$(dirname "$(realpath "$(command -v node)")")")
mkdir -p "$destination"
docker run --rm --user "$(id -u):$(id -g)" --workdir /source \
    --mount "type=bind,source=$source_dir,target=/source" \
    --mount "type=bind,source=$inputs/$arch,target=/inputs,readonly" \
    --mount "type=bind,source=$scratch/packages,target=/packages" \
    --mount "type=bind,source=$scratch/home,target=/build-home" \
    --mount "type=bind,source=$HOME/.cargo,target=/cargo" \
    --mount "type=bind,source=$HOME/.rustup,target=/rustup,readonly" \
    --mount "type=bind,source=$node_dir,target=/node,readonly" \
    --env HOME=/build-home --env CARGO_HOME=/cargo --env RUSTUP_HOME=/rustup \
    --env PATH=/cargo/bin:/node/bin:/usr/bin:/bin --env CARGO_INCREMENTAL=0 --env CARGO_BUILD_JOBS=2 \
    "$image" bash -euo pipefail -c '
        pacman -Q glibc gtk3 webkit2gtk-4.1
        python3 scripts/package-linux-desktop.py --manifest "/inputs/$1.json" --archive "/inputs/$1" --output /packages
    ' bash "$archive"
shopt -s nullglob
packages=("$scratch"/packages/*.pkg.tar.xz "$scratch"/packages/*.pkg.tar.zst)
[[ ${#packages[@]} == 1 ]]
cp -- "${packages[0]}" "$destination/"
