ARG UBUNTU_VERSION=24.04
FROM ubuntu:${UBUNTU_VERSION}

ARG GO_VERSION=1.25.0
ARG WEBKIT_VARIANT=webkit2_41

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH=/usr/local/go/bin:/root/go/bin:$PATH
ENV GOMODCACHE=/go/pkg/mod
ENV GOCACHE=/go/build-cache
ENV CGO_ENABLED=1

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    build-essential \
    dpkg-dev \
    libgtk-3-dev \
    pkg-config \
    $(if [[ "${WEBKIT_VARIANT}" == "webkit2_41" ]]; then echo "libwebkit2gtk-4.1-dev"; else echo "libwebkit2gtk-4.0-dev"; fi) \
  && rm -rf /var/lib/apt/lists/*

RUN arch="$(dpkg --print-architecture)" \
  && case "${arch}" in \
    amd64) go_arch="amd64" ;; \
    arm64) go_arch="arm64" ;; \
    *) echo "unsupported Go download architecture: ${arch}" >&2; exit 1 ;; \
  esac \
  && curl -fsSLo /tmp/go.tgz "https://go.dev/dl/go${GO_VERSION}.linux-${go_arch}.tar.gz" \
  && rm -rf /usr/local/go \
  && tar -C /usr/local -xzf /tmp/go.tgz \
  && rm -f /tmp/go.tgz \
  && go version

WORKDIR /workspace
