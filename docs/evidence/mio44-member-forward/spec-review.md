# MIOPUNCH-44 Spec review

Reviewed committed change `108f74c5bb9eaffa4d2eb6bce7ab77e6f06eb8d7...b0629e98ab167f42d9e34d2f901e3845ae212da9` against the live MIOPUNCH-44 description, its MIOPUNCH-14 parent specification, current domain notes and ADR 0005. Original dirty documentation and configuration were excluded from the change under review.

**Missing or partial requirements:** No findings within MIOPUNCH-44's scope. The real three-member checks demonstrate protected A-C control through existing A-B/B-C connections, normal A-C business after the same connection gains a permitted UDP path, and rejection when no other data path is permitted while A-B/B-C remain usable.

**Scope creep:** No findings. The implementation adds one member message adapter and generalizes the existing MQTT bootstrap coordinator. It does not add member business relaying, arbitrary multi-hop routing, roles, new authorization rules or a separate connection-attempt engine.

**Implemented-looking but wrong behavior:** No findings. Inner A-C TLS authentication and member admission remain active; B rewrites the source token from its authenticated registration rather than accepting a caller-supplied identity. Member transport is admitted only for restricted setup and is excluded from normal data carriers. B's registration count, frame size, queues and stream lifetime are bounded; expiry, interruption and endpoint closure retire the forwarding stream without closing the independent outer member connection.

Evidence inspected: `tests/member_forward.rs`, `scripts/check_member_forward.py`, `final/results.json`, `final/mqtt-short/results.json`, their relevant logs, and the implementation note. The runner orders the first real check before the priority failure check. Both existing final receipts report `passed:true`; this review did not rerun them. Loopback acceptance and packet-marker observations are not broader NAT or cryptographic-audit claims. Unchanged slow NAT matrices were skipped under the user's policy.

Spec findings: **0**.
