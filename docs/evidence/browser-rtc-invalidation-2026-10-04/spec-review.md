# STEP8 independent SPEC review — Sol6.1 / HIGH

Reviewed frozen staged tree `270e7127629430db811abf2ffb3ff69b0d9f8054` against released STEP7 base `221950480e92e8b4b29515a74566ccab5693e0a1`, using the full current browser WebRTC replan, AGENTS, CONTEXT and relevant accepted ADRs. Read-only review; no builds/tests/traffic/Plane or repository writes.

**Findings: 0 (no severity assigned).**

(a) Missing/partial requirements: none identified within the authorized STEP8 standard contract.

(b) Scope creep: none identified. Controlled SDK/resource/Connection seams and the narrow post-FINAL probe are authorized. Full browser rotation, global instant atomic revocation and a successful delayed-notification experiment are not STEP8 prerequisites.

(c) Implemented requirements with wrong behavior: none identified.

**Sufficiency: PASS for scoped STEP8; no eleven-step closure.**

The originating contract requires “每次只改变一种条件” (replan:153), cancellation followed by a fresh successful attempt (154), invalidation between ACK and release without old-M resurrection (155), and observable cleanup beyond “一次快照检查” (156). The actual runner applies per-attempt cancellation then exchanges data on a distinct M (run-rtc-invalidation.mjs:124–194), signed SDK removal, durable current-credential installation, and installed carrier permission independently (228–278). Retired routes/queues/pumps and physical sends remain retired across actual captured callbacks and closed-PC full-SDP rejection (333–377). Secret generation installation uses save_admission_material/current store pin, not fabricated revision state (native_network.rs:1003–1030; tests/rtc_invalidation.rs:70–107).

Replan:26 requires one remaining deadline; 53/55 require fresh exact-M/path ACK and current qualification. Both actors renew after private FINAL (src/rtc_member.rs:539–574; browser-probe/src/rtc_member.rs:327–343). validate_release observes current IO and the existing caller clock (rtc_transport.rs:518–560); final actor authorization rechecks the exact route (vendor/iroh/src/socket/remote_map/remote_state.rs:901–908). The independent real-QUIC component keeps original UDP M, permission and old ACKs valid while new RTC renewal times out, then exchanges an actual stream (tests/rtc_release_validation.rs:104–171).

Verified all 36 archive content/compressed hashes, 52 current source hashes, and five executed artifacts. Standard secret receipt records watermark 9→16/unacknowledged/no handle; normal actors record 10→17 and 5→13/fresh ACKs. Physical counters freeze after local retirement; the browser’s pre-retirement validation packet is accurately qualified. Six fixtures exit zero with owned-resource absence proof within 60 seconds. The failed controlled-cleanup prerequisite remains explicitly unqualified (invalidation note:51), separate from standard stories and component proof. No further fix/recheck is required for this frozen tree.
