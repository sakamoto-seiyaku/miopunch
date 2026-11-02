# Read-only physical WAN prerequisites

2026-10-05. [MIOPUNCH-82](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7178d29a-2fc8-45a4-b972-8ab23dd39588).
This is environment evidence, not a task ledger or SDK/public-NAT pass. No device,
host VPN, gateway configuration or network was changed. The ticket remains
unclaimed pending root confirmation of a controllable, qualified two-end site.

## Observed site

| Endpoint | Physical local address | Control | Bounded public observations |
| --- | --- | --- | --- |
| WSL host | `192.168.4.5/24`, `eth2`, gateway `192.168.4.11` | local | Normally verified source-bound TLS: `104.28.195.192`, `warp=on`; one source-bound UDP STUN: `104.28.227.187:64094` |
| Y700 / TB322FC | `192.168.0.107/24`, `wlan0`, gateway `192.168.0.1` | existing network ADB over Tailscale | Normally verified source-bound TLS: `183.197.94.96`, `warp=off`; shell public STUN destination route uses WLAN |
| Pixel 6a | `192.168.4.151/24`, `wlan0` | existing USB ADB | Same host LAN; not an independent WAN endpoint |

The default ADB wrapper and `/usr/local/bin/adb` call a Windows executable that
fails with exit 126 in this session. Explicit Linux ELF ADB
`/home/js/Android/Sdk/platform-tools/adb` version 37.0.0 works with both devices.
Device serials, hardware addresses and complete connectivity dumps are omitted.

Y700 has active `WIFI|VPN` on `tun1` as well as physical `WIFI` on `wlan0`.
Its VPN routing table includes `192.168.4.0/24`, so a host candidate for the
workstation can select Tailscale despite the two different physical WLANs. The
old [Y700 RTC receipt](../y700-public-rtc-2026-10-04/receipt.json) selected exactly
that overlay path; its interoperability result is retained under that scope.

The host's normal TLS observation explicitly reports WARP. The UDP observation
records its own socket source and mapped address, not an RTC candidate pair.
Neither an HTTP WARP flag nor an address-prefix comparison proves how a future
RTC/business flow will route. The current site has not qualified physical NAT.
Android shell UID route lookup also does not prove the browser application's UID
or its selected socket route.

## Reproducible startup, observation and cleanup

The entry is [wan_environment_preflight.py](../../../scripts/wan_environment_preflight.py).
It needs Python, RTK, `ip`, `curl` and, for Android, an existing authorized native
ADB connection. It does not connect new devices, start servers, create forwarding,
install artifacts or change networking. Run with authorized host netlink/ADB
access; sandbox restrictions are separately retained below.

```sh
rtk proxy python3 scripts/wan_environment_preflight.py \
  --output /tmp/miopunch-wan-preflight-fresh.json \
  --host-bind 192.168.4.5 \
  --adb /home/js/Android/Sdk/platform-tools/adb \
  --android-model TB322FC --android-bind 192.168.0.107 \
  --stun 74.125.250.129:19302
```

Use the current observed physical addresses, not these addresses blindly after
a site change. Both HTTPS requests use normal certificate verification, explicit
source addresses and no configured HTTP proxy. Android argv is quoted as one
remote shell command so `--noproxy '*'` remains literal. Each curl has a 6-second
cap. The optional STUN operation sends exactly one request with a 3-second cap;
it checks response endpoint, header, transaction and XOR-mapped address. It does
not configure TURN or declare native RTC gathering fixed.

The script records original commands (device selector redacted), actual exit codes
and elapsed time. Parsing failure is separate and never overwrites the exit code.
Each local command runs in its own session; a local 12-second wait timeout sends
TERM then, if needed, KILL to its owned process group and waits for the leader.
The receipt distinguishes forced cleanup, actual leader exit and immediately
observed group absence. It does not claim forced termination was graceful, nor
that killing a local ADB group proves a remote child exited. Remote curl has its
independent 6-second cap. The sole owned UDP socket is closed in `finally`.
STUN caller and cleanup times are separate. No SDK listener, PC,
pump, queue or asynchronous socket teardown is created, so their lifecycle is
outside this receipt. There are no owned forward/reverse rules to remove; do not
use global ADB removal or `kill-server` to clean up a future test.

Exit 0 means prerequisite commands completed; it never means physical-WAN pass.
Exit 2 records a command/observation failure or rejects invalid arguments. An
existing output is refused to preserve earlier failures. The receipt always has
`physicalWanQualified=false`. A future product run must independently report
permission gate, identity-match booleans, actual selected tuples, original QUIC
fresh ACK, business/replica behavior, deadline and its own cleanup/late socket
observation; this entry cannot stand in for that run.

## Evidence and verification

- [preflight.json](preflight.json): corrected entry exit 0; all commands exit 0;
  host STUN caller 190.020ms, independent cleanup 0.025ms, owned socket closed;
  each local group observed absent immediately after its leader exited.
- [sandbox-and-wrapper-failure.json](sandbox-and-wrapper-failure.json): entry
  exit 2; netlink permission failures and Windows wrapper exit 126 retained.
- [observed-failures.json](observed-failures.json): bounded SSH aliases failed
  (`vps_nl`/`vps_hk` banner timeouts, `server` no route), original quoting timeout,
  and the overwrite guard's exit 2. No remote process or file was intentionally
  created through SSH.
- [identities.json](identities.json): entry/native ADB/evidence SHA256 identities.
  HEAD is separately recorded in the live receipt. No SDK binary ran here.
- [focused-validation.json](focused-validation.json): nonzero exit 42 retained
  without parsing; malformed success output retains exit 0 plus a separate parse
  failure; a deliberate local parent/child timeout has actual exit -15, forced
  TERM cleanup 1.485ms and observed group absence.

The original entry receipts and their identities are preserved under `initial-*`.
Their earlier JSON parser masked failing `ip` exit codes as null and their cleanup
statement was broader than measured. The corrected receipts above supersede
those claims while retaining the initial observations and failures.

The live run, sandbox failure, overwrite rejection and focused local failure
checks exercise the preparation entry's success and evidence/cleanup boundaries.
No old implementation test suite was rerun.

## Site condition still required

Y700 is already on a different WLAN and its observed physical TLS exit is not
WARP; moving it is unnecessary on current evidence. The remaining site question
is a controllable second endpoint/exit whose tested data route can be proven
independent of WARP/other overlays. Existing SSH aliases did not provide one in
the bounded checks. A human may provide another endpoint or change the host's
physical access/exit; this agent must not disable whole-machine VPN or alter
gateway policy. Alternatively, retain VPN solely for control only after root
confirms that per-test binding, selected peer tuples and independent routing or
capture prove the actual SDK data path is physical.

The existing `RtcIceProfile.candidate_addresses` restricts published/accepted SDP
after gathering; it does not bind browser sockets or defeat upstream WARP.
Native `peer_with_profile` also defaults to `0.0.0.0:0` at this checkpoint. Later
runtime changes belong to their implementation tickets; no profile with guessed
public candidates is generated here.
