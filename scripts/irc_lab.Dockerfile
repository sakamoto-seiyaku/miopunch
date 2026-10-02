FROM ergochat/ergo@sha256:cf50544214cf196d25913553a451b766101edce114b5e005a7c063da578851a7 AS ircd
FROM miopunch-nat61-lab:bookworm
COPY --from=ircd /ircd-bin /ircd-bin
COPY --from=ircd /lib/ld-musl-x86_64.so.1 /lib/ld-musl-x86_64.so.1
