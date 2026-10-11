# RTC repeat after the user switched Pixel Wi-Fi

The same pinned Android/WASM SDK artifacts now pass the full public-MQTT
admission, reopen, nonempty Replica and ordinary RTC business story with
OP13/PJZ110 on cellular and Pixel_6a Chrome on the user's newly selected
Wi-Fi. No data relay or TURN is configured. The SDK selects RTC before and
after actual PING/PONG on the original ordinary member connection.

This repeat follows the
[Wi-Fi UDP destination diagnosis](../rtc-wifi-udp-destination-diagnosis-2026-10-11/README.md).
The Pixel IPv4 address and default gateway changed; OP13's cellular binding
and default gateway stayed unchanged. The agent changed no network settings.
The previous failure's controller and capture helper are byte-identical;
the driver changes only the output directory. All 559 frozen SDK source
files, selected dependency inputs, pinned artifacts and the 234 protected
working files match their earlier receipts. Cargo/rustc were not invoked.

| Actual operation | SDK caller time | Controller wall time |
|---|---:|---:|
| Nonempty Replica synchronization | 8,579.680 ms | 8,692.811 ms |
| Ordinary member connection | 7,857.570 ms | 7,894.186 ms |
| PING → PONG exchange | 44.510 ms | Not separately staged |

The complete test, including setup, approval, persistence reopens, negative
checks, captures and owned cleanup, took 74,103.828 ms. This is not a
controlled performance comparison between Wi-Fi networks or transports.

The initial restricted request, real approval/material delivery, both SDK
identity reopens, four-record Replica verification/save/apply and Browser
reopen persistence passed. The ordinary handle is authorized for the
expected peer, uses strict purpose 1 with fresh final ACK, retains its
original local connection and binding, and exchanges actual PING/PONG.
Unapproved and actually revoked data requests remain rejected.

Before the SDK story, the same independent raw UDP probe that failed on the
previous Wi-Fi passed all six destination checks across the two phones.
Pixel now receives the second target's reply when changing destinations
on the same socket. Fresh-socket controls also pass. The original failure
records remain failures; this is a new successful test after the user's
network change, with the product code and artifacts held constant.

The two pre-SDK captures now include peer STUN responses with
MESSAGE-INTEGRITY attributes and actual DTLS handshake/application packets.
The existing route collector reports physical data and WAN qualification
for this ordinary member connection: native selected-tuple ownership and
observed socket mark, exact routes to the current cellular/Wi-Fi interfaces,
nonrelay selected pairs, and bidirectional exact ICE payload correlation.
The correlation matches five Native→Browser and three Browser→Native ICE
requests. It does not independently validate MESSAGE-INTEGRITY, and actual
selected socket UID observation remains zero; process/package UID and
Browser socket protection are not substituted for unknown socket facts.
See `route-proof.json` for the retained qualification limits.

All owned SDK actors, capture processes, Browser tab, storage, ADB rules and
local listeners were retired through the existing cleanup. Native exit is
zero without force, Browser storage deletion completes under its ownership
lock, capture drops are zero and neither capture hits its packet limit.
Native SDK reports zero active RTC associations/pumps after close, but still
four sockets at the immediate and 100 ms observations. This repeat does not
claim complete SDK terminal socket retirement or a complete direction matrix.

The previous MQTT and both relay cases' captures contain the same old Pixel
Wi-Fi address. Both successful relay cases selected data relay; allowing RTC
did not establish successful RTC business. The IRC case likewise reports a
relay business path and RTC role `None`. IRC has no corresponding packet
capture in this comparison, so its exact Wi-Fi address is not inferred from
packet evidence. These results are not evidence that other handshakes got
lucky with the first ICE candidate.

Published files contain selected scalars, enum facts, input source and hashes.
Actual identities, grants, secrets, route endpoints, SDK stores and raw packet
payloads remain outside the repository. `raw-references.json` binds the
retained run artifacts; `sdk-binding.json` and `artifact-reuse.json` bind the
reused SDK graphs and executed inputs. No prior evidence was relabeled.
