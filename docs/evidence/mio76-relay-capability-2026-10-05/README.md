# Browser RTC capability and permitted relay evidence

Actual peers are Pixel6a browser and Y700 native SDK. The host builds, controls
ADB and records evidence. Contact uses explicitly configured EMQX MQTT TLS 8883
and WSS 8084 with normal certificate validation. The data relay is the explicitly
permitted APS1 iroh HTTPS relay; no TURN or independent STUN probe is configured.

[disabled-first](disabled-first/receipt.json) preserves the original exit 1:
manual admission, durable authorization/secret and both identity reopens passed,
but the member call exceeded its original 12-second deadline. Its cause remains
unresolved; the initial relay WebSocket `CONNECTING` observation does not identify
a network or protocol fault.

[disabled-diagnosis-1](disabled-diagnosis-1/receipt.json) exits 0 with the same SDK
artifacts and budget. RTC policy disables API access: support remains unknown,
permission and effective capability are false, and PC/DC creation counts are zero.
A configured, disabled observer does not gather. The strict new member session
advertises no RTC role, verifies both identities and authorization gates, obtains
fresh authenticated ACKs and selects the permitted relay. The original session
exchanges and echoes 50 bytes; connect takes 6808.845 ms. Existing owned-page CDP
records actual MQTT and relay WSS 101 responses without frames or headers.

Both receipts include commands, exits, source/artifact hashes and cleanup.
The successful case closes SDK handles, deletes its owned IndexedDB, removes the
owned page and ADB rules, ends readers and the local CDP observer, and restores
native socket counts to the baseline at immediate and late observations.

The successful disabled case matched all 473 native build inputs at its run.
The immutable WASM
build retains its full historical manifest and publishes the six approved native
module deltas alongside 478 matching inputs; these unused native branches are not
qualified by this endpoint-only browser story. This evidence qualifies the
explicitly disabled RTC case and relay stream, without establishing public NAT
RTC, other runtime failure causes, or the no-data-path restoration story.

[missing-api-first](missing-api-first/receipt.json) and the sole authorized
[bounded retry](missing-api-bounded-retry-1/receipt.json) both preserve exit 1.
They use a freshly compiled native artifact with the full-facility permission
fix: all 474 current native inputs match. The actual 186-entry WASM dependency
inventory remains unchanged; the additional native-only delta is published.
The owned-page injection produces `api-missing`, support false, permission true,
effective false and zero PC/DC. Admission and durable identity reopen pass, but
member calls expire after 12106.39 ms and 12097.54 ms respectively. Both retain
the same running member, saved materials and ready MQTT contact after failure.
The retry relay WSS remains open and exchanges data, so these observations do
not establish a facility or common-protocol cause. All owned resources retire.
Further API-missing public attempts are held pending narrow diagnosis; these
receipts qualify runtime detection and cleanup, without qualifying its member
relay stream.
