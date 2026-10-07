# Spec

0 actionable findings. The frozen 46-file change and the recorded functional evidence satisfy the reviewed MIOPUNCH-77 and MIOPUNCH-78 acceptance criteria within their declared SDK scope. This review does not close the wider MIOPUNCH-69 goal.

M77 asks for “在 RTC 准备前原 stream 已传数据” and “fresh ACK/资格复查/双方确认后同 M 同 stream 继续”. Actual public-relay and explicitly authorized MID1 cases show business before preparation, current bilateral gates, fresh initial/final probes and common commit, then PONG on the original M/stream. Quality retains the measured better relay; PreferDirect selects qualified RTC. The three real negative cases preserve healthy relay business after actual probe loss, a measured worse RTC path and candidate-driver failure. The long retention case proves successful paths survive both setup ceilings. Local stable IDs remain endpoint-local; exporter equality is represented by matching booleans.

M78 asks that B “不等自身 relay 失败就响应受限联系” and that “旧 generation 回调不能重新发布，禁用设施无 socket”. Actual member-first/public-MQTT fallback observations show A without relay, B's relay healthy before and after, strict admission and data/PONG. Normal SDK reconnect/withdrawal, the explicitly identified private generation regression, and stopped-origin query/Replica retirement cover lifecycle behavior without a second reconnect loop. Original independent M business survives disable and real persistence failures.

I read seven M77 cases, ten M78 receipts, six affected shared/root regressions, final Native/WASM build receipts and formatter lineage. Final verification matches all 46 captured/live files, 555 frozen sources, 20 case-receipt hashes and pinned artifacts. I executed no tests or runtime actors.

Declared limits remain: Host SDK/local RTC plus real public facilities, not physical WAN; Browser quiescent CLOSING tails and opaque SDK task retirement are not called graceful. Owned resources/processes and immediate/late descriptors retire without forced termination. M80/M81/M82/M84 remain outside this review.
