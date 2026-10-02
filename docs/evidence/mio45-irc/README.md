# MIOPUNCH-45 evidence

The behavioral contract and supported configuration are in
[the validation note](../../notes/2026-10-02-irc-bootstrap-validation.md).
Run `rtk proxy python -B scripts/check_irc_bootstrap.py --output <new-directory>`;
add `--short` for the affected suite. Each receipt contains exact commands,
exit codes and logs. The owned IRCd is real Ergo 2.7.0, pinned by image digest;
TCP fault proxies change wire delivery only. QUIC/TLS, membership checks,
native stores and path release remain real.

`first-real-red/test.log` is the genuine first behavioral RED: saved IRC
configuration did not advertise a route. The real IRCd was listening; its
startup receipt is linked alongside that log. `first-green/test.log` is the
first protected IRC-to-UDP connection and actual ordinary business GREEN.
`priority/test.log` follows that first success and drops final fragments of
real long QUIC handshakes, proving bounded failure and no business release.
`reassembly/test.log` observes 32 retained incomplete groups, 48 overflow
rejections and complete expiry through the public adapter interface.

`identity-integrity-probe/test.log` contains targeted, temporary opaque packet
hash observations: received datagrams exactly match sent datagrams, excluding
reassembly corruption as the retry failure's cause. `identity-wrong-only/`,
`identity-accept-loop/` and `identity-size-probe/` retain the narrowed REDs.
`identity-queue-ttl/test.log` is the narrowed GREEN after bounded outgoing
datagram age; `identity-final-fix/test.log` restores both wrong TLS identity and
unapproved membership negatives followed by successful legitimate contact.
All temporary diagnostic code was removed from source.

The initial `first-red/` is a sandbox socket permission failure, and
`first-behavioral-red/` ran while the initial IRCd fixture still had an invalid
TLS listener. Neither is the genuine behavioral RED. `data-cut/` records a
missing musl loader, corrected in the image. `data-cut-runtime-fix/`,
`data-cut-flow-fix/` and `data-cut-delivery/` show why cutting only selected
initial UDP ports did not remove all paths: ordinary iroh candidate expansion
also supplies other UDP paths. `data-cut-all-owned-udp/` is the actual GREEN
with all IPv4/IPv6 UDP removed in its owned offline namespace. No host network
rules were altered. No business-byte dump represents private user data; that
diagnostic payload is the fixed synthetic byte 0x42.

`lifecycle/test.log` retains two earlier failures. Cancellation was observed
before TLS registration, where connection metrics correctly remain zero;
`cancel-stage-fix/test.log` moves the test to the already approved public
before-data-release hook and verifies closure of the original authenticated
attempt plus a new stable ID. The ordinary accept loop alone did not fix the
identity retry RED; queue expiry was necessary. The final matrix and independent
review reports below supersede those earlier incomplete runs.

`final/results.json` records the first ordered nine-case matrix and real isolated
data-cut, followed by a short-suite format failure while a test was still being
edited. The behavioral short groups, all-targets and WASM checks passed before
that failure; they are not falsely counted as a successful whole receipt.
`continued-short/results.json` completes formatting, nine member-carrier tests,
all current hook compilation and default native compilation.
`completed-acceptance/results.json` is the authoritative ordered ten-case IRC
matrix plus actual isolated data-cut on the final source. Both continued and
completed receipts report passed=true. The affected existing short groups
report 137 passing tests, including inert subprocess worker entries. Slow already verified unchanged
NAT matrices are excluded per the user's standing instruction. WASM checks are
compile-only. No new arbitrary-NAT, public-service or other-platform claim is made.
