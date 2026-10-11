# M88 Standards review

Fixed review range: `b696ecbba54733277d2562e1a8a7ab78767dc494...e1a48295fb6ad1c8eba509cb72fd9ed0b0084b08`.

No actionable documented-standard violations or baseline code smells found. Severity: none.

Reviewed AGENTS.md, RTK.md, build-cache, domain and issue-tracker rules, CONTEXT.md and ADR 0009. The small iroh change confines periodic quality probes to non-Control connections; it preserves ordinary path scoring and the existing QUIC mechanisms. The shared MIR2 codec owns encoding, fragment bounds, actual prefix/target line budgeting, reassembly and qualification. Native and browser adapters consume that contract.

Read-only revalidation matched all 16 published probe-source hashes to their exact private originals, the current Native/WASM build receipts and immutable artifacts, 6590/3159 actual selected dependency inputs, and copied repository sources. The 39-test receipt, executed binary hashes and 5734 actual Native selected inputs also match; the final browser dependency edge did not change that graph. External Cargo.lock package rows are unchanged.

Raw public and owned SDK receipts match published hashes and the critical identity, purpose, approval, durability, Base64, relay-business and natural-cleanup assertions. Six actual SDK cases remain qualified. The five-PING Control red result, zero-PING passing assertion, ordinary probing, slower 268-second pacing-only result, and initial failed TLS shutdown remain represented accurately. The 60.488-second result explicitly uses a 300-second caller budget and does not claim default-60-second success.

Published receipts/probe sources preserve the stated scalar/hash privacy boundary; generated identities, secrets, stores and raw ciphertext remain private. Whole-handshake/CID/echo reductions are correctly deferred and unclaimed. Protected CONTEXT/ADR WIP is outside this owned diff.

No compilation, test execution, public traffic, product edits or staging performed.
