# MIOPUNCH-32 Standards review

Required findings: **0**. Optional findings: **1**.

Optional judgement — possible **Duplicated Code**, `src/daily_management.rs:157` and `:232`: both use `.nodes().into_iter().find(|(node, _)| *node == target).map(|(_, access)| access)`. A small store access lookup could keep the before/after membership query consistent as the representation changes. This is a maintenance suggestion, not a documented-standard violation or acceptance blocker.

The changes follow the current baseline in AGENTS.md and docs/agents/domain.md. The integration choice and limits are explicitly recorded in the validation note. ADR 0007's original signature, causal-prestate verifier, independent root/device authors and StrongRemove semantics remain in use; ADR 0008's root-plus-canonical-Create credential is checked. Session ownership and expiry govern local signing only. Durable receipt/replay does not trust a forwarded session or local boolean. No archived workflow gates or silent product decisions were introduced.

Reviewed fixed baseline: `8ee52120b6499b5ff70142aba571e2bc43c7ce2d`; no intervening commits. The nonempty cached diff covers exactly the 26-file whitelist. All cached and working-file hashes match `/tmp/mio32-plane/reviewed-staging.json` (SHA-256 `5e7f557aa44c6cfb16b5c24b07db1b1eb6f330b5843c09a2faa21bf4ba52c4f0`). Exact changed source/harness SHA-256:

- daily_management.rs: `8d698590ed1bd3786fe1297d92ade4fac39d7ef75373cafa0d378ee785c77cad`
- genesis_recovery.rs: `a845ee1a9397fb8c94316390b36416a55de6037fbc044d90008597d744c8f25c`
- lib.rs: `5c6ad2826f9673c95a4528a77125855daed7e89d99966cf22624f8982e512d37`
- management_store.rs: `1c4e0ae4c71e0d93e0fec3f69ef8019efd7a9e9c0f03f8fa75192ddfb63876d9`
- native_network.rs: `8ad677d8da73a53f512f3617435daed069d8d8019a971f7025a8509372ecff36`
- tests/daily_management.rs: `6241ceb9694b1b9c89375e98a6badee995ac6d0a2d6e0174eda080d439a800d9`
- scripts/check_daily_management.py: `5cb914fb721c7a357e9ecee033893dcf6c394a94c7a7aa106ec8085815a540ce`

Verification: ten receipt source hashes and all 59 protected paths match; all 13 gzip logs match original/compressed/roundtrip hashes and byte counts. Existing generation-1 results show 274 passed, 0 failed, 4 existing ignored, six management tests, all five checks exiting 0, and 92 actual MQTT publishes. Cleanup readback confirms the owned broker absent. No suites were rerun. Runtime evidence is native Linux loopback; browser evidence is compilation only. This report addresses Standards only.
