FROM debian:bookworm
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    iproute2 iptables conntrack tcpdump mosquitto python3 ca-certificates procps \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /lab
