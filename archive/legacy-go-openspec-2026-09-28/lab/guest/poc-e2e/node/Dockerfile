FROM debian:12

ENV container=docker
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    dbus \
    iproute2 \
    iputils-ping \
    jq \
    less \
    netcat-openbsd \
    procps \
    systemd \
    systemd-sysv \
    tcpdump \
    tmux \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/* \
  && ln -sf /dev/null /etc/systemd/system/dev-hugepages.mount \
  && ln -sf /dev/null /etc/systemd/system/sys-fs-fuse-connections.mount \
  && ln -sf /dev/null /etc/systemd/system/systemd-logind.service \
  && ln -sf /dev/null /etc/systemd/system/getty.target \
  && ln -sf /dev/null /etc/systemd/system/getty@.service

COPY bin/miopunch /usr/local/bin/miopunch
COPY bin/miopunch-poc-e2e /usr/local/bin/miopunch-poc-e2e

RUN chmod 0755 /usr/local/bin/miopunch /usr/local/bin/miopunch-poc-e2e \
  && mkdir -p /var/lib/miopunch /run/miopunch /artifacts

STOPSIGNAL SIGRTMIN+3
VOLUME ["/sys/fs/cgroup"]

CMD ["/lib/systemd/systemd"]
