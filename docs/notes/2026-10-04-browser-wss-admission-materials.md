# First browser admission through owned MQTT-WSS

STEP10 of the [current WebRTC plan](2026-10-03-browser-webrtc-replan.md) now has a real empty-applicant path. The browser starts without membership authorization or admission secret, authenticates the invitation's contact on restricted `miopunch/admission-request/1`, and receives material through the existing manual/precode protocol. The administrator approves the exact authenticated `(applicant, request)`; approval-all and injected browser material are not used.

Original signed history, canonical genesis, contact administrator and the applicant's current grant are verified before the authorization snapshot is committed. The secret arrives privately from that contact with the pinned generation. A strict IndexedDB comparison/replacement transaction completes before the loaded gate is installed and SAVE is sent. The public redacted material receipt checks the actual leased store and exposes only identities, hashes, generation and save facts. Sender Complete records its authenticated SAVE receipt; local admission success requires both truly saved materials. Neither observation claims global convergence.

The request, retained Delivery and AUTH/receive/save/SAVE/DONE share one absolute caller deadline. Native providers respect the original incoming deadline and their existing ceiling. An AwaitingApproval return followed by an explicit poll/reopen is a new attempt with the same persisted namespace; an ongoing delivery cannot renew its clock. Browser12/contact10/maintenance2 remain unchanged. A timeout after a successful save does not roll back material.

The actual final run measures these separate boundaries:

- EMPTY applicant → selective approval → PendingMaterials → real secret transaction completion → Complete; same-origin/profile SDK reopen preserves identity and exact signed grant. A then closes. A **new**, independently proved Member M obtains its own initial and newer release ACKs, RTC seal and real PING→PONG. This is not A changing into M.
- Authorization-only page/SDK interruption preserves PendingMaterials. A one-shot real IndexedDB transaction abort prevents secret save/SAVE. The original authenticated applicant then resumes the same request/grant at the original node without another approval/Add.
- An independently keyed holder sends the approved request ID on its own actual TLS A: its own claim receives unapproved status, while claiming the other identity is rejected. It receives no history/secret. Durable identity/request/materials are never edited to construct these claims.
- A limit1 precode's authorization-only Delivery actually expires; completion fails promptly rather than starting another12s. Native DATA_DIR and browser profile reopen preserve the original result. Resume consumes no additional use; a different holder's exhausted new redemption is refused. Public trusted usage/redemption reads remain1/1 with the same authorization.
- After real Complete, an Open DataChannel drops a1200B QUIC datagram. The original2s Member attempt fails without an ordinary handle or signaling-data fallback. Reopening the same profile still yields identical Complete material. This failed new M is separate from A and any later new attempt.

Final qualification uses Chromium/Linux native in one owned namespace with normal TLS/private NSS, explicit MQTT routes, no IP/data-relay bootstrap and empty TURN/ICE servers. Material history is tiny and causally complete. Each scenario's120s includes startup and owned cleanup; compilation is separately timed. Final material protocol28.759s; affected actual STEP9 nonempty replica/save-abort/reopen23.017s, on the same five fresh artifacts. Actual PC callbacks/observers, queues, pumps and routes close; still-running native sockets return to baseline6. These are measured quiescence receipts, not universal instantaneous socket-zero claims or WAN/NAT qualification.

The original-deadline public native test failed before the fix and passed after it; its real TLS/remaining-budget red is kept separately from earlier sandbox bind/Docker setup failures. Current checks have123 library passing entries plus1 existing ignore and77 admission/proof/gate/precode/completion passing entries; the latter include existing inert crash-worker entries, while the configured MQTT cases really run. Native/WASM, formatting and syntax checks pass. Review status and complete original/archived/source/artifact provenance are in the [evidence package](../evidence/browser-wss-admission-materials-2026-10-04/README.md).

Reproduce into a new directory:

```sh
rtk proxy python3 scripts/check_browser_paths.py --case wss-admission-materials --output /tmp/mio-rtc-s10-new
```

IRC's stopped restricted-handshake profile remains unqualified. No UI/IWA, alternate crypto, synchronization protocol, default resource/backend, required TURN, full rotation, unknown-history recovery or migration scope was added. STEP11 remains separate; this result does not complete the full eleven-step objective.
