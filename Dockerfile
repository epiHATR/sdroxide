# SDRoxide headless server image (browser UI on :4950).
#
# Install modes (build-arg INSTALL_FROM):
#   local    — unpack artifacts/sdroxide-<label>.tar.gz from the build context
#              (default; used by .github/workflows/docker.yml against *this*
#              repo's release assets — never pulls from upstream)
#   release  — curl a GitHub release tarball; you must pass SDROXIDE_REPO and
#              SDROXIDE_VERSION for *your* fork, e.g.:
#                --build-arg INSTALL_FROM=release \
#                --build-arg SDROXIDE_REPO=you/sdroxide \
#                --build-arg SDROXIDE_VERSION=v1.6.11
#
# Prefer *-compat Linux tarballs: debian:bookworm-slim is glibc 2.36.

FROM debian:bookworm-slim

# Default is local so a plain `docker build` / CI never downloads
# dividebysandwich/sdroxide (or any other remote) by accident.
ARG INSTALL_FROM=local
ARG SDROXIDE_VERSION=
ARG SDROXIDE_REPO=
# Overridden per-arch by BuildKit's TARGETARCH in the install step when unset.
ARG SDROXIDE_LABEL=
ARG SDROXIDE_SUFFIX=
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        libusb-1.0-0 \
        libudev1 \
    && rm -rf /var/lib/apt/lists/*

# SoapySDR runtime only needed for the -soapysdr artifact.
RUN if [ -n "$SDROXIDE_SUFFIX" ]; then \
      apt-get update && apt-get install -y --no-install-recommends soapysdr0.8-module-all \
      && rm -rf /var/lib/apt/lists/*; \
    fi

WORKDIR /opt/sdroxide

# CI drops tarballs here as artifacts/sdroxide-<label>.tar.gz
COPY artifacts/ /tmp/artifacts/

RUN set -eux; \
    arch="${TARGETARCH:-amd64}"; \
    if [ -n "$SDROXIDE_LABEL" ]; then \
      label="$SDROXIDE_LABEL"; \
    else \
      case "$arch" in \
        amd64|x86_64) label="linux-x86_64-compat" ;; \
        arm64|aarch64) label="linux-aarch64-compat" ;; \
        *) echo "unsupported TARGETARCH=$arch" >&2; exit 1 ;; \
      esac; \
    fi; \
    short_tgz="/tmp/artifacts/sdroxide-${label}.tar.gz"; \
    if [ "$INSTALL_FROM" = "local" ]; then \
      if [ -n "$SDROXIDE_VERSION" ]; then \
        pkg="sdroxide-${SDROXIDE_VERSION}-${label}${SDROXIDE_SUFFIX}"; \
        local_tgz="/tmp/artifacts/${pkg}.tar.gz"; \
      else \
        local_tgz=""; \
      fi; \
      if [ -n "$local_tgz" ] && [ -f "$local_tgz" ]; then tgz="$local_tgz"; \
      elif [ -f "$short_tgz" ]; then tgz="$short_tgz"; \
      else \
        echo "INSTALL_FROM=local: place release tarballs in artifacts/" >&2; \
        echo "  expected $short_tgz (or full name with SDROXIDE_VERSION)" >&2; \
        ls -la /tmp/artifacts >&2 || true; \
        exit 1; \
      fi; \
      tar -xzf "$tgz" -C /opt; \
    elif [ "$INSTALL_FROM" = "release" ]; then \
      if [ -z "$SDROXIDE_REPO" ] || [ -z "$SDROXIDE_VERSION" ]; then \
        echo "INSTALL_FROM=release requires SDROXIDE_REPO and SDROXIDE_VERSION" >&2; \
        echo "  e.g. --build-arg SDROXIDE_REPO=you/sdroxide --build-arg SDROXIDE_VERSION=v1.6.11" >&2; \
        exit 1; \
      fi; \
      pkg="sdroxide-${SDROXIDE_VERSION}-${label}${SDROXIDE_SUFFIX}"; \
      url="https://github.com/${SDROXIDE_REPO}/releases/download/${SDROXIDE_VERSION}/${pkg}.tar.gz"; \
      echo "Downloading $url"; \
      curl -fsSL "$url" -o /tmp/sdroxide.tgz; \
      tar -xzf /tmp/sdroxide.tgz -C /opt; \
      rm -f /tmp/sdroxide.tgz; \
    else \
      echo "INSTALL_FROM must be 'local' or 'release' (got '$INSTALL_FROM')" >&2; \
      exit 1; \
    fi; \
    test -x /opt/sdroxide/sdroxide; \
    rm -rf /tmp/artifacts

ENV HOME=/data \
    XDG_CONFIG_HOME=/data/.config \
    PATH="/opt/sdroxide:${PATH}" \
    SDROXIDE_SONDE_MOD_DIR=/opt/sdroxide

# Do not force GIDs: dialout/plugdev already exist on Debian with their own
# IDs. Do not pass --system with --uid 1000 — SYS_UID_MAX is 999, so that
# combination exits 1 on bookworm.
RUN mkdir -p /data/.config/sdroxide \
    && getent group plugdev >/dev/null || groupadd plugdev \
    && getent group dialout >/dev/null || groupadd dialout \
    && useradd --home-dir /data --shell /usr/sbin/nologin \
         --uid 1000 --user-group -G dialout,plugdev sdroxide \
    && chown -R sdroxide:sdroxide /data

USER sdroxide
WORKDIR /data
EXPOSE 4950

ENTRYPOINT ["sdroxide", "--server", "--port", "4950"]
