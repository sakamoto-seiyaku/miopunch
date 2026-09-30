FROM miopunch-nat59-lab:bookworm
RUN apt-get update -qq \
 && apt-get install -y --no-install-recommends nftables \
 && rm -rf /var/lib/apt/lists/*
