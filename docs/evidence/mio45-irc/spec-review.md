# MIOPUNCH-45 independent Spec review

Reviewed committed diff `2b3711ab5a9af39a560902f52bcf8f6f46cd65b9...4c4b5dc387a56f9d968d2d49cf33cb098052131e` (one commit). Sources: live Plane MIOPUNCH-45, parent MIOPUNCH-14 and its approved Testing Decisions, CONTEXT, accepted ADRs 0004/0005. Existing user WIP and `.cargo/config.toml` are outside this review.

**Findings: 0.**

- **Missing/partial requirements:** none identified within the declared qualification. “将同一连接入口从 MQTT 换成真实 IRC 服务并完成引导” is covered by configured native binding, the shared bootstrap hook, real membership/TLS, and matching bootstrap/data stable IDs in `completed-acceptance/first.log`. Long actual QUIC handshakes, missing/repeated fragments, bounded reassembly, actual server fakelag, fresh binding and cancellation have separate acceptance cases in `tests/irc_bootstrap.rs`.
- **Unrequested scope creep:** none identified. The carrier, integration, resource filtering, runnable fixture and evidence support this ticket. No UI, KCP, service ACL or general runtime plugin platform is added.
- **Implemented-looking but wrong behavior:** none identified. “业务不在 IRC 上传输” and “旁听/注入负例及数据路径失效后的用途限制与 MQTT 分支一致” retain the shared admission/data-release boundary. Independent IRC observation, wrong-target/unapproved-member rejection, and real isolated IPv4/IPv6 UDP loss substantiate the tested boundary. `completed-acceptance/data-cut.log` preserves the original stable ID and records unchanged IRC publication counts after queued business fails.

`completed-acceptance/results.json` and `continued-short/results.json` report success; the earlier failed whole short-suite receipt is explicitly retained as failed. This review inspected code and existing execution evidence without rerunning tests. Qualification remains Linux loopback, real pinned Ergo with the documented service settings, and UDP data release. Arbitrary public IRC services, server TLS, transparent session reconnect, broader NAT/platform support and a cryptographic audit are not claimed or inferred. Fresh reconnect is distinct from the original session, consistent with the documented lifecycle outcome.
