# Spec review — MIOPUNCH-43

Reviewed independently by GPT-6.1 Sol HIGH Fast against the staged owned work
from b36ad3d98d7ac07458170ed89408f45190c91948 and fetched MIOPUNCH-43/14.
No tests rerun or source edits by reviewer.

Initial review: **1 P1 — one-sided TCP observation failure prevents permitted
UDP fallback.** Ticket requires “代表性不可打通拓扑有界失败，获准备用可继续”.
The original `tcp_punch.rs` returned before candidate exchange on observer
failure, while the peer still expected MIOTCP01. The next bi stream crossed
TCP/UDP formats and UDP rejected it. Real asymmetric-observer-red reproduced
that JSON parse failure and original-session cancellation.

Targeted re-review: **0 remaining Spec findings.** The previous P1 is resolved:
peers now exchange protected observation availability, candidate permission,
and validation results before switching to UDP or releasing TCP data.
The asymmetric-observer-fixed-green and candidate-permission-green receipts
confirm permitted UDP fallback on the original connection with no pending TCP
sockets. No additional missing requirements, scope creep, or incorrect behavior
confirmed.

Final affected suites were still running at reviewer readback. Their actual
completion is recorded separately in review-fix-acceptance/results.json and
review-fix-short/results.json.
