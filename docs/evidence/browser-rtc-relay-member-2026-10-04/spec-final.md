# Final Spec recheck — S5 WIP

Same baseline: `f5bae5c727c632faf5cac0b3473b01cbbe938fe4`, tracked WIP and the six specified untracked files. Read-only source/evidence inspection; no builds, tests, network traffic, or Plane access.

**Both required P2 findings resolved; no remaining required S5 findings.**

The original native-pump terminal defect is fixed by `RtcPumpGuard::drop` (`src/rtc_transport.rs:390`), which retires exact IO on every task exit. Native creates the guard before fallible work (`src/rtc_member.rs:146`) and observes pump completion during coordination (`:513`). The provided GREEN receipt records inactive/unqualified retirement, zero pumps/queues, sockets 6→6, 9ms failure return, and continued business on the same original relay M.

The held-datagram defect is fixed by shared `RtcDatagramIo::while_active` (`src/rtc_transport.rs:402`). Its biased selection checks retirement, original-M closure, and independently notified qualification changes before polling held work. Resource retirement wakes the registered waiter; revision notifications wake idle/backpressured drivers. Both actual pumps use this seam and recheck current IO immediately before channel send (`src/rtc_member.rs:188`, `browser-probe/src/rtc_member.rs:107`). This satisfies the common spec boundaries “资源禁用…都要退休 lease、关闭或拒绝路径” (line 55) and “关闭唤醒所有等待者” (line 202). Packet callbacks retain independent atomic/weak-liveness checks and do not re-enter gate or connection locks.

The targeted regression uses an actual TLS M and dequeued QUIC path packet, simultaneous resource retirement/capacity readiness, and separate qualification/original-close cases. I inspected its assertions; execution results are supplied by the parent. Final affected suites and final real source-matched runtime remain the parent's validation gates, not independently executed by this reviewer.

Final compatibility readback confirmed: `permits_custom_connection` and `permits_custom_application_data` apply extra checks only to opt-in carriers; `qualifies_custom_data` delegates the latter. RTC opts in by transport ID even after retirement, so foreign-M routing, pending ordinary-data scheduling, and authenticated Noq receive rejection remain strict. Legacy path scoring and terminal-send handling retain their previous behavior. Existing custom allowlist, retired-address gates, and resource-bound send checks remain composed separately; this change grants no signaling-data permission. Retained RTC lifetime remains independent of the attempt deadline. Both P2 fixes remain intact. Remaining required findings: **0**; no scope creep or optional suggestions. Steps 6–11 and broader roles remain outside S5.

Final fixture readback: `browser_restricted_native.rs` now waits for browser A closure after a successful admission reply, using the same original `deadline` defined at incoming start. No grace period or deadline renewal is added. Core admission/RTC behavior and unchanged independent-identity/request assertions remain intact; the supplied full S2 GREEN receipt passes those assertions. This narrow fixture correction adds no product scope. Required findings remain **0**.

Finding history: `/tmp/miopunch-rtc-s5-spec-review-20261004.md` and `/tmp/miopunch-rtc-s5-spec-recheck-history-20261004.md`.
