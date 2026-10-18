# Ordinary endpoint compatibility evidence

MIOPUNCH-95 restores ordinary incoming early-data and original Accept/Reject hooks while retaining the separate restricted admission pipeline. Implementation: `d4789c6ae6c9434c448ffd24575baba44b119413`, baseline `ba5f544a55536789b17278646785a4d0fde01a46`. See [implementation note](../../notes/2026-10-19-ordinary-endpoint-compatibility.md).

- [Validation](validation.json): unchanged official/fork consumers, original fork failure, four ordinary runtimes, 36 distinct affected runtime tests, 18 compiler contracts, final Native/WASM builds and four SDK scenarios.
- [Independent review](review.md): separate Standards and Spec reports, both with zero outstanding findings.
- [Raw verification archive](raw-verification.tar.gz): 207 driver, selector, fixed-consumer, compiler/log, selected-input and receipt files. Every member was read back and verified against [the hash manifest](archive-manifest.json).

The fixed ordinary consumers were qualified on official iroh before production edits, failed unchanged on the fork, then passed unchanged after restoration. Final build inputs and immutable SDK/test artifacts were checked again after all runs. The environment-gated historical protocol counterpart ran explicitly; its initially ignored selection is not counted as a pass.

The SDK cases cover durable admission and identity reopen, member traffic, original-member Relay to RTC improvement, restricted negative cases, and Browser incoming manual approval/precode/retry/replica/RTC. Local departure prevents local delivery; a peer that has not learned departure is assessed under the existing propagation window. All actors close naturally; owned containers and generated private keys/NSS are removed.

Private SDK identities, SDP, stores and raw SDK logs remain local. Published SDK results use an explicit scalar/hash allowlist. Binaries/WASM and selected source copies remain under local receipt paths. One obsolete intermediate test binary is losslessly compressed; its compression receipt records the exact original hash and restoration path. Official baselines, final tests and SDK pins remain intact.

This is affected owned-loopback qualification; it does not complete broader M91 work or the independent public/physical-device matrix. All 249 pre-existing WIP files retain their bytes. No push is performed.
