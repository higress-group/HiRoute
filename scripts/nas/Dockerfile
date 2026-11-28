# HiRoute standalone runtime image for Docker Compose (NAS or headless host).
#
# Build context is this folder. Copy the verified HiRoute standalone archive
# here first (see README.md), then build with docker compose.
#
# Required build args:
#   ARCHIVE        - standalone tar.gz filename inside the build context
#   ARCHIVE_SHA256 - sha256 of that archive
#   CPA_SHA256     - sha256 of the bundled cliproxyapi binary
#
# The archive is rearranged to match the official installer layout:
#   /opt/hiroute/hiroute, /opt/hiroute/hirouted, /opt/hiroute/console,
#   /opt/hiroute/libexec/cliproxyapi, ...
# so the daemon's install-record identity check passes.

FROM debian:bookworm-slim

ARG ARCHIVE
ARG ARCHIVE_SHA256
ARG CPA_SHA256

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 1000 --home-dir /var/lib/hiroute --shell /usr/sbin/nologin hiroute

COPY ${ARCHIVE} /tmp/hiroute.tar.gz
RUN printf '%s  /tmp/hiroute.tar.gz\n' "${ARCHIVE_SHA256}" | sha256sum -c - \
 && mkdir -p /opt/hiroute \
 && tar -xzf /tmp/hiroute.tar.gz -C /opt/hiroute \
 && mv /opt/hiroute/bin/hiroute /opt/hiroute/hiroute \
 && mv /opt/hiroute/bin/hirouted /opt/hiroute/hirouted \
 && rmdir /opt/hiroute/bin \
 && rm -f /tmp/hiroute.tar.gz \
 && ln -sf /opt/hiroute/hiroute /usr/local/bin/hiroute \
 && ln -sf /opt/hiroute/hirouted /usr/local/bin/hirouted \
 && mkdir -p /run/hiroute \
 && chown -R hiroute:hiroute /run/hiroute

ENV HOME=/var/lib/hiroute \
    XDG_STATE_HOME=/var/lib/hiroute/state \
    XDG_DATA_HOME=/var/lib/hiroute/share \
    XDG_RUNTIME_DIR=/run/hiroute \
    CPA_SHA256=${CPA_SHA256}

COPY entrypoint.sh /usr/local/bin/hiroute-entrypoint.sh
RUN chmod 0755 /usr/local/bin/hiroute-entrypoint.sh

EXPOSE 5838

ENTRYPOINT ["/usr/local/bin/hiroute-entrypoint.sh"]
CMD ["hirouted"]