FROM ubuntu:22.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 curl tmux ca-certificates openssl && rm -rf /var/lib/apt/lists/* && useradd --uid 1000 --create-home --shell /bin/bash demo
ENV LANG=C.UTF-8
WORKDIR /app
CMD ["python3", "/app/server.py"]
