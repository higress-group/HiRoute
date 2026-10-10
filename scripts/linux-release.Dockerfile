# The caller supplies the digest-pinned image and immutable Debian snapshot.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG APT_SNAPSHOT
RUN printf 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/%s/ bullseye main\n' "$APT_SNAPSHOT" > /etc/apt/sources.list \
    && printf 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/%s/ bullseye-security main\n' "$APT_SNAPSHOT" >> /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/* \
    && apt-get update \
    && apt-get install --yes --no-install-recommends cmake clang pkg-config binutils \
    && rm -rf /var/lib/apt/lists/* \
    && test "$(getconf GNU_LIBC_VERSION)" = 'glibc 2.31'
