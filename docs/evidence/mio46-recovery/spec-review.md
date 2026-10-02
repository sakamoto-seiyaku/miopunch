# MIOPUNCH-46 independent Spec review

Reviewed `c6134e27098981fa9754b8265d47c91d083b729e...95b536d0f5284a86fb145a66d8333800dc7ebf17`, using the complete MIOPUNCH-46 and parent MIOPUNCH-14 snapshots, current CONTEXT, ADRs 0004/0005/0007/0008, the validated discovery profile and recorded recovery integration choices. Uncommitted user documents and `.cargo/config.toml` are excluded.

**Findings: 0.** No concrete missing/partial requirement, unrequested behavior, or incorrectly implemented requirement found within this ticket's declared native Linux MQTT/IPv4 profile.

The requirement “A 不知 C 身份、B 全程离线，A 自动发现 C 并经真实数据路径取得需要的状态，进入正常范围” is exercised through `resume_network`, independent persisted identities, actual MQTT discovery, the original authenticated connection's UDP path, reused LogSync and subsequent authorized business communication. The reopened-startup check verifies C's authorization survives A's store reopen.

“只把候选当线索；保留资源资格、阶段准入和真实身份核对” is reflected in Lookup+Signaling resource qualification, origin-tag validation, exact TLS EndpointId, session-bound admission proof and source membership checks before any history manifest. The fresh identity's old secret grants Bootstrap but neither history nor Member access. Wrong network/generation, missing material, false candidates and known revocations have dedicated checks.

“候选失效、入口部分失败、重复线索和缺依赖均可恢复或明确等待” is covered by retry on the same endpoint, scoped contact backoff, duplicate suppression and `WaitingForDependencies` retaining saved/applied source progress. `Ready` requires durable source completion and current authorization, with no global freshness claim.

Reviewed retained acceptance/regression receipts and relevant test assertions; no tests rerun during this review. The earlier resource regression is retained and its corrected check passes. Evidence does not establish broader NAT, relay-only unknown recovery, automatic IRC discovery or other-platform support, and the implementation documentation does not claim them.
