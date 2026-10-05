# Unified Browser→Native qualification

The complete formal Pixel6a browser → Y700 native member story passed with
exit 0 in [full-member-success.json](full-member-success.json). Both members
saved authorization and admission material, reopened their independent stores
with unchanged identities, and used `miopunch/session/member/1`. Current gate
checks, the original QUIC session and exporter binding, bilateral fresh ACKs on
the exact RTC route, and PING/PONG all passed. The member connection took
10,936 ms within its existing 12-second budget; business exchange took 65 ms.
Native MQTT TLS on port 8883 and browser WSS on 8084 used normal certificate
verification.

The browser's selected UDP pair was public srflx `111.197.237.227:37644` to
`183.197.94.96:26729`. The native socket was physically bound at
`192.168.0.107:54041`, with the peer's public srflx endpoint. No TURN was
configured. These are actual RTC pair and data facts. A contemporaneous
UID/fwmark policy-route or packet-capture proof was not collected, so
`physicalWanQualified` remains false; this does not qualify the full platform
matrix or claim an operating-system route proof.

Cleanup returned native socket FD count from active 9 to baseline 6 at both
the immediate and 100 ms late observations. Counts include inherited socket
descriptors; they are not an absolute zero or unique-socket count. The MQTT
driver, queues, RTC association and pump retired, the actual PC close
succeeded, and the owned process exited 0 without forced termination. Browser
SDK associations and pumps were zero immediately and at 100 ms; its owned
database, page, forwarding rules and HTTP controller were removed.

The first formal case exited with code 1 during
native startup, before a browser page, member gate or RTC candidate started.
The native process exited about 1.5 seconds after startup. Its owned process,
readers and temporary directory were absent after cleanup. The original receipt
is [first-startup-failure.json](first-startup-failure.json).

The follow-up used the same frozen native binary, facility and approved ICE
profile for native startup only. Its actual driver error was
`invalid peer certificate: UnknownIssuer`; the original native exit remained 1.
The receipt is [native-startup-diagnosis.json](native-startup-diagnosis.json).

Both receipts requested native MQTT TLS on `broker.emqx.io:8883` and browser
normal WSS on port 8084. The configured `normalTlsVerification` and
`nativeOuterTls` flags describe that requested verification, not a successful
TLS qualification. No member, QUIC data path, RTC or physical WAN success is
claimed by these failures.

The pinned native MQTT client discovers Android roots through the Termux
certificate path. The ordinary foreground shell has no Termux certificate
store. The adapter's Android default was corrected to use embedded Mozilla
roots, matching the existing native IRC and iroh public-root configuration;
certificate and hostname verification remain enabled. Explicit custom CA
configuration and other operating systems retain their existing behavior.

[native-startup-after-trust-fix.json](native-startup-after-trust-fix.json)
records the same startup passing in 2,812 ms. A subsequent full attempt stopped
at an unrelated 5,283 ms MQTT network timeout before any browser or member
connection; [second-network-timeout.json](second-network-timeout.json) preserves
that exit 1. One bounded retry then completed the full story. No product budget
or pacing was increased.

Nine affected SDK checks passed in [nine-sdk-current.log](nine-sdk-current.log).
The controlled native RTC abort test reached real answer allocation, then
proved caller cancellation and candidate timeout retire the association,
withhold ordinary handles, reject a late answer on a closed fixture PC and
preserve another healthy stream. It uses a controlled protected wire peer;
it does not claim browser callback qualification. The stale-attempt check
rejects an incorrect protected RTC context before allocation.

Frozen native and browser build receipts, complete compiled-source hash
snapshots, the original-to-stripped deployment chain and complete browser
package hashes are retained. The broader old WASM snapshot contains later
native-only source changes; the runtime receipt records their scopes instead
of claiming that the entire current native library matches that browser build.

No independent public STUN qualification, TURN, product deadline increase or
plaintext SDP/signaling protocol was used. Published receipts omit raw SDP,
exporter values, credentials and private keys. The completed Browser→Native
story qualifies this software direction; physical route and remaining platform
matrix qualification remain separate.

A subsequent Standards review found that an explicit MQTT contact override
could borrow another allowed facility's permission when both used the same
native URL. The actual SDK bind regression reproduced this for an A facility
disabled by WSS or logical ID: its driver and broker socket still opened.
[contact-override-red.log](contact-override-red.log) preserves exit 101 and
successful cleanup before the final failing assertion.

The native entrance now requires the complete canonical saved and allowed
facility descriptor. The transport retains that descriptor separately from
its existing service tag. A raw legacy contact keeps the explicit URL policy
for unmapped resources and cannot borrow a mapped facility's consent.
[contact-override-green.log](contact-override-green.log) records rejection
before driver/socket creation, while B at the same native URL remains usable.
The full affected SDK file then passed 10 checks in 7.29 seconds, including
an unsaved descriptor and a permitted logical ID with a changed WSS endpoint;
[ten-sdk-after-contact-policy.log](ten-sdk-after-contact-policy.log) preserves
the result. An explicitly permitted unmapped legacy URL remains usable and its
disabled form is rejected. All owned sockets retire.

[contact-policy-fix.json](contact-policy-fix.json) records the exact updated
source hashes and this local qualification. The public story used one uniquely
allowed facility, whose authorization outcome is unchanged. Its immutable
Android artifacts and historical receipts remain intact; these source changes
have not been rebuilt or rerun on the phones. No public or STUN test was
repeated for this correction.
