# MIOPUNCH-65 Spec review

Result: **0 findings** across missing/partial requirements, unasked scope, and incorrect implementation.

Reviewed the supplied live-ticket HTML and the five-path WIP diff against `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`; no ticket commits exist yet. Shared MIOPUNCH-64 and unrelated WIP are excluded from this review.

“实际浏览器运行当前 BrowserNetwork，使用独立持久身份和 IndexedDB” is satisfied by the existing BrowserNetwork and a separate persistent Chromium profile. The static origin only distributes assets. The runner passes no custom CA, sets `ignoreHTTPSErrors: false`, and supplies no certificate-ignore argument. The selected hostname remains listed in [official source](https://github.com/n0-computer/iroh/blob/main/iroh/src/defaults.rs); [official documentation](https://docs.iroh.computer/add-a-relay) identifies these n0 public relays as development/testing facilities.

“端口连通或 WebSocket 打开不能单独作为完整成员通信成功” is satisfied by exact authorized echo responses, native selected-relay path observations with bidirectional bytes, and real native-source LogSync. The runner verifies the original nonempty signed-record body, both saved admission materials, and the same identity/record after refresh. Retained current-entry evidence records Chromium 153.0.8010.12, origin, timestamps, HTTPS results, and three actual WSS 101 sessions.

“仅阻断或禁用选定官方 relay，且不给未批准的替代设施” is exercised after the first complete check: explicit and persisted disable fail clearly, with disabled business controls, zero WSS/external attempts, and a measured 334 ms first failure. Explicit restore and reopen retain identity and the exact record and perform another real exchange. Unchanged member/request/record counts and one approval demonstrate no repeated Add, approval, or redemption.

No product scope expansion was found. Native changes add bounded fixture startup and transport observations. Original RED, public GREEN runs, affected checks, cleanup, and the separate self-hosted regression are retained. Tests were not rerun during this read-only review. The concurrent shared-core build context and unchanged slow NAT exclusion are explicitly disclosed.

Review aggregation, ticket commit, and final OS-temp handoff remain normal post-review completion steps, not behavioral defects.

Reviewer runtime readback: GPT-6.1 Sol, HIGH, actual `priority` service tier.
