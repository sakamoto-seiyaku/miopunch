## 1. Formal Documentation

- [x] 1.1 Update Door 2 TCP punching charter with TCP assisted/private candidate semantics and F-002/F-005 test implications.
- [x] 1.2 Update P3 transport charter with peer transport session, logical stream, and stream/session close semantics.
- [x] 1.3 Update Door 3 signaling backend charter with exchange schedule versus punching phase schedule boundaries.
- [x] 1.4 Update mainline network findings to reference the finalized design conclusions and follow-up change IDs.

## 2. OpenSpec Synchronization

- [x] 2.1 Add punching-decision requirements for backend-neutral phase scheduling and success-only analyzer memory.
- [x] 2.2 Add tcp-p2p requirements for TCP assisted candidates and assisted-only fallback.
- [x] 2.3 Add dataplane requirements for peer transport sessions and generic logical streams.
- [x] 2.4 Add MQTT signaling requirement that exchange readiness does not encode NAT role timing.

## 3. Verification

- [x] 3.1 Run `openspec validate formalize-mainline-connectivity-fixes`.
- [x] 3.2 Confirm no Go code, lab scripts, or runtime behavior changed in this change.
