# Spec review — MIOPUNCH-37

Reviewed the authorized WIP against baseline `a6e384ca9d19595721a1b373ebde47feb21efade`, the complete original MIOPUNCH-37/parent MIOPUNCH-14, ADRs 0004/0007/0008 and the accepted MIOPUNCH-36 lifecycle. Findings below are static code findings; no new runtime experiments were run.

## Required corrections

- **P2 — A later rotation strands a retained member missing the previous completion.** Requirement: “收到未提交、提交后确认丢失及发起端重启均可幂等恢复，保留成员重启后仍能使用新秘密。” At [admission_rotation.rs:560](../../../src/admission_rotation.rs#L560), the initiator may begin generation 2 immediately after committing generation 1. If a retained member saved/ACKed generation 1 but missed its completion, reopening leaves it active at generation 0 with generation 1 staged. Distribution selects only the newest row ([line 250](../../../src/admission_rotation.rs#L250), [line 805](../../../src/admission_rotation.rs#L805)); that member rejects generation 2's `previous_generation` at [line 1032](../../../src/admission_rotation.rs#L1032). Generation 1 completion is no longer publicly deliverable, while ordinary resumption is paused. Restore delivery of the exact predecessor completion before advancing that member, without relaxing signatures, generation or membership checks. Add a public-node regression for ACK → missed completion → initiator starts next rotation → both reopen → successful catch-up.

- **P2 — Pending retained delivery requires the initiator to be its original provider.** Requirement: “创世提权启动轮换，保留成员可靠保存并确认后才切换”; “分发替换秘密可继续。” Let provider A approve X, interrupt before X saves the old secret, synchronize X's valid membership to elevated device B, then start rotation at B. B retains X but lacks A's local pending-delivery row ([line 670](../../../src/admission_rotation.rs#L670)); X also pins A as contact ([line 974](../../../src/admission_rotation.rs#L974)). A cannot distribute B's proposal because [line 808](../../../src/admission_rotation.rs#L808) requires the sender to be the initiator. Enable original-provider replacement distribution/ACK coordination for this case, preserving the exact proposal, current grant and fresh original root proof where applicable. Cover ordinary and pending-root recipients through public behavior.

## Optional findings

None. No unrequested product expansion identified.

## Evidence

Final1 source/archive/frozen-executable hashes match; wrapper/all 13 commands exited 0; logs show 84 passes, 13 genuine child exits 87 and real MQTT evidence. Protected files and frozen NAT evidence are unchanged. The 43 NAT omissions have unchanged source and no new-name collisions; two affected credential/scope cases ran. Linux/WASM/power-loss limitations are accurately stated. These passing scenarios do not cover the two gaps above.
