ARG BASE
FROM ${BASE}
ARG KEYRING
ARG MIRROR
ARG BUILD_UID
ARG BUILD_GID

# Match the Omarchy stable Arch repositories and require trusted package signatures.
# The hosted ARM container cannot apply the downloader Landlock rules.
RUN if [ "$KEYRING" = archlinuxarm ]; then set -- --disable-sandbox-filesystem; fi \
    && pacman-key --init \
    && pacman-key --populate "$KEYRING" \
    && sed -i -E 's/^[[:space:]]*SigLevel[[:space:]]*=.*/SigLevel = Required DatabaseOptional/' /etc/pacman.conf \
    && printf 'Server = %s\n' "$MIRROR" > /etc/pacman.d/mirrorlist \
    && pacman -Syyuu --noconfirm --needed "$@" base-devel clang git gtk3 libarchive python systemd webkit2gtk-4.1 \
        wl-clipboard xdg-utils xdg-desktop-portal xz zstd \
    && pacman -Scc --noconfirm \
    && (getent group "$BUILD_GID" >/dev/null || groupadd -g "$BUILD_GID" builder) \
    && (getent passwd "$BUILD_UID" >/dev/null || useradd -M -u "$BUILD_UID" -g "$BUILD_GID" builder)
