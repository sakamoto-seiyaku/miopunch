# Build tools and a current Windows API implementation are separate so that
# the exact Windows artifact also remains usable outside the Wine runtime.
FROM debian:bookworm AS compiler
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gcc-mingw-w64-x86-64-posix ca-certificates && rm -rf /var/lib/apt/lists/*

FROM debian:trixie AS runtime
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    wine64 ca-certificates && rm -rf /var/lib/apt/lists/*
