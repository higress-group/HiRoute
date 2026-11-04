# The caller supplies the digest-pinned image and immutable Debian snapshot.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG APT_SNAPSHOT
# Fetch signed indices before admitting package mirrors. APT verifies package
# files against the hashes in those pinned indices.
RUN printf 'https://snapshot.debian.org/archive/debian/%s/\tpriority:2\nhttps://snapshot-cloudflare.debian.org/archive/debian/%s/\tpriority:3\n' \
        "$APT_SNAPSHOT" "$APT_SNAPSHOT" > /etc/apt/bullseye-mirrors \
    && printf 'https://snapshot.debian.org/archive/debian-security/%s/\tpriority:2\nhttps://snapshot-cloudflare.debian.org/archive/debian-security/%s/\tpriority:3\n' \
        "$APT_SNAPSHOT" "$APT_SNAPSHOT" > /etc/apt/bullseye-security-mirrors \
    && printf 'deb [check-valid-until=no] mirror+file:/etc/apt/bullseye-mirrors bullseye main\ndeb [check-valid-until=no] mirror+file:/etc/apt/bullseye-security-mirrors bullseye-security main\n' > /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/* \
    && apt-get -o Acquire::Retries=3 update --error-on=any \
    && printf 'https://deb.debian.org/debian/\tpriority:1 type:deb\n' >> /etc/apt/bullseye-mirrors \
    && printf 'https://deb.debian.org/debian-security/\tpriority:1 type:deb\n' >> /etc/apt/bullseye-security-mirrors \
    && apt-get -o Acquire::Retries=3 install --yes --no-install-recommends cmake clang pkg-config binutils ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && test "$(getconf GNU_LIBC_VERSION)" = 'glibc 2.31'

# Official checksums: https://nodejs.org/dist/v22.19.0/SHASUMS256.txt
# The policy selects the native archive; host Node binaries are not mounted.
ARG NODE_VERSION
ARG NODE_ARCH
ARG NODE_SHA256
RUN curl --fail --location --retry 3 --proto '=https' --tlsv1.2 \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz" \
        --output /tmp/node.tar.xz \
    && printf '%s  /tmp/node.tar.xz\n' "$NODE_SHA256" | sha256sum --check --strict \
    && tar --extract --xz --file /tmp/node.tar.xz --directory /usr/local --strip-components=1 --no-same-owner \
    && rm /tmp/node.tar.xz \
    && test "$(node --version)" = "v${NODE_VERSION}" \
    && npm --version
