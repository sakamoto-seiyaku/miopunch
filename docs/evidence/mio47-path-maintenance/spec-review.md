# MIOPUNCH-47 independent Spec review

Reviewed committed `56d40e0ee07dee6a2ed0ec6a9115ed26c59be60d...2dde359b1ae7a5acf821c64cdf72bae210e3ea04`, using the captured MIOPUNCH-47 and MIOPUNCH-14 specifications, current domain notes and ADR-0005. Protected user WIP is excluded.

**One finding — P2, implemented requirement with an incorrect success condition.**

The ticket requires “分别验证已有备用路径、无预建备用但经控制入口在原连接存活窗口内协调新路径、原连接已关闭三种情况”; the parent says “如果原连接最终超时、关闭或无法维持，向调用方明确报告失败，不伪装成仍可传输。”

In [src/connection_recovery.rs:187](../../../src/connection_recovery.rs#L187), `validate_connection_candidates` reports success whenever an offered address occurs in `original.paths()`. For an unchanged-address physical outage, the previously established path can remain listed during the original QUIC connection's timeout window. The peer's offer can repeat that address. [open_path_on_conn](../../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1581) immediately accepts an already-open address without fresh validation, so recovery can return `Ok(ConnectionPathRecovery)` while the only business path still cannot carry bytes. The close check distinguishes a terminated connection but cannot prove recovery on a locally living one.

Require bounded fresh confirmation on the original data path, or distinguish newly established paths from pre-attempt entries and verify any reused entry before reporting success. Add a real unchanged-address physical-loss case with live control; it should remain retryable while the original lives and become terminal on its actual close, rather than report recovery solely from the old list entry.

The final thirteen-case evidence remains valid for its declared Linux IPv4 loopback matrix. Its no-backup proxy cases offer a different reachable endpoint address, so they do not exercise this branch. No other missing/partial requirement or unasked behavior was found: RTT policy, budgeted reevaluation, explicit resource restrictions, per-connection eligibility, two-sided protected repair and new-identity failure evidence fit the ticket. Broader NAT/platform support is explicitly unclaimed and is not treated as missing acceptance.

Review used source and recorded evidence; no tests were rerun and no runtime/test files were edited.

## Final supplement — f6817654881f364348794a5c5191fa2172a5fdb8

Reviewed correction `2dde359b...f6817654` and the resulting MIOPUNCH-47 implementation against the same ticket and parent requirements. **The P2 finding above is resolved; zero remaining Spec findings.**

`validate_connection_candidates` now starts one fresh probe for each qualified offered path and requires its acknowledgment before returning success. `noq::PathProbe` retains the exact original connection/path. The protocol captures the next unused data packet number while queuing PING under the same connection lock; earlier acknowledgments cannot satisfy that watermark. The existing authenticated, per-path ACK processing supplies the observation. Abandoned paths and connections outside the established state reject both probe creation and acknowledgment checks, including before packet draining completes. Current membership and original close checks remain decisive.

The real [same-address RED](same-address-red/control_paths_do_not_report_stale_same_address_as_recovered.log) demonstrates the original false `Ok`. The [final fourteen-case matrix](final-fresh-ack-acceptance/results.json) passes healthy unchanged-address confirmation, physical loss returning Retryable while the original lives, and Terminal after its actual close, alongside the prior same-stream, identity, quality, restriction and isolation cases. [Protocol checks](fresh-ack-protocol-checks/results.json) and [immediate-close evidence](fresh-ack-immediate-close-check/run.log) cover old ACK rejection, distinct paths, abandonment and closed connections; final library/type/WASM/format receipts pass.

No new missing/partial requirement, scope creep or incorrect success condition was found. The probe uses ordinary QUIC traffic and existing ACK handling; it adds neither a new wire handshake nor application recovery. Declared Linux loopback qualification and compilation-only browser evidence remain accurately bounded.

This supplement inspected source and recorded results only. No successful checks or unchanged slow NAT matrices were rerun; no runtime/test files were edited. STOP.
