# STEP10 + STEP11 union — independent Standards review

**Required: 0. Hard documented-standard violations: 0. Optional: 1 (P3).** No P0/P1/P2 finding or required correctness fix.

Reviewed the complete staged source/helper/test changes against fixed base/HEAD `a48dbe0af23cb03dadf53bc3778cea6dfee6fca0`, plus staged metadata and binary archive ledgers. Standards sources: root `AGENTS.md`, `/home/js/.codex/RTK.md`, `CONTEXT.md`, `docs/agents/domain.md`, current direction/scenario notes and ADR0004/0005/0006; Fowler smell baseline from `/home/js/.agents/skills/code-review/SKILL.md`. Tooling-enforced style is excluded.

The changes respect the documented domain and decision boundaries. STEP10 admission A, original Delivery clock, verified durable materials/Complete and subsequent new M remain distinct. The optional bounded ICE profile neither grants membership nor changes global resource-origin/default rules; saved observer URL disables remain checked. Original-M/current qualification and exact-path release remain the actor boundary. No archived Go/OpenSpec gates, product adoption, TURN/default-service decision or broad framework change is introduced.

**Optional P3 — possible Duplicated Code:** `src/bin/browser_probe_native.rs:21` and `:26` repeat the same certificate/key loading shape:

```rust
let certificate = rustls::pki_types::CertificateDer::from(std::fs::read(certificate_path)?);
let key = rustls::pki_types::PrivatePkcs8KeyDer::from(std::fs::read(key_path)?);
```

The new bind/host CLI form duplicates the existing file-based form. A small shared certificate-loading function could prevent future loading/validation changes drifting between forms. This is a maintenance judgment, not a correctness issue or acceptance prerequisite.

Fresh evidence keeps historical isolated qualification separate, preserves actual failures, limits each story to 120 seconds including cleanup, reports RTC's slower finite comparison, and qualifies shared-host CPU/RSS and encrypted outer-packet limitations. Recorded checks support 307 passing native entries plus one existing ignore; inert workers are not claimed as additional business stories. Formal acceptance/Plane/full-goal completion remain root-owned.

Read-only review only: no build, test, protocol, container, traffic, Plane, source/index/config or runtime mutation. Only this external report was written.

## Exact provenance

- Command: `rtk proxy git diff --cached a48dbe0af23cb03dadf53bc3778cea6dfee6fca0 -- .`; `git log a48dbe0..HEAD --oneline` is empty by design.
- Full binary diff: 1,016 paths, 3,793,352 bytes; SHA256 `21266781f42c823a77b526c6ccdb35d5a1a15c0409f4a9974c6cc000dfce7111`.
- Index tree reconstructed without Git writes: `8c8745f57d0c03ae1c1e0e453f79b5e665897432`. All 1,016 changed working-file blobs match staged identities.
- All 85 source hashes and five artifact hashes match pinned input/current receipt. Static explicit registry equals the prior 68/76 union of 85 existing files, including `src/lib.rs`; clean-commit provenance does not depend on WIP discovery.
- Independently checked all 389 fresh and 582 historical original/stored/gzip identity triplets; no mismatches or missing originals. All 591 copied isolated evidence paths match accepted `fc87779f0baad182a786d220464c629a3dfdd013` blobs.
- Fresh archive transformation audit: 24 pcaps/8,126 records retain exact original permitted headers, timestamps and wire lengths with payload stripping; 60 JSON documents preserve 234 ordinary nulls and original numeric process codes. Fresh local metadata links resolve.
- Fresh receipt SHA256 `9f1fde90b6a5ccde5df5b91c35d3926bfd8f11dba87fa4b35b636b8ad5246d93`; archive-audit SHA256 `61fbf504fbf465abcfbaa6b6786aa9d8c81735230c4627598c884ee306793be7`.
- Reviewer `/root/webrtc_s10_sol61_high/s11_union_standards`; thread `01a10714-bff3-7f02-b162-4cf1f3635fde`. Same reviewer remains available for narrow fixes/final publication audit.
