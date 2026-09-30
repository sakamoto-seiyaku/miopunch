FROM miopunch-nat57-lab:bookworm
RUN apt-get update -qq \
 && apt-get install -y --no-install-recommends python3-pip python3-dev gcc libnetfilter-queue-dev \
 && pip3 install --break-system-packages NetfilterQueue==1.1.0 \
 && rm -rf /var/lib/apt/lists/*
