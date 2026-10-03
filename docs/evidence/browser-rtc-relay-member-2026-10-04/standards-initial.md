# S5 Standards review

Read-only review on 2026-10-04. Fixed point and HEAD both resolve to
`f5bae5c727c632faf5cac0b3473b01cbbe938fe4`; commit list since fixed point is empty.
The target is WIP, including untracked sources.

## Required findings

None found. No documented hard standards breach identified.

The repository requires current domain/ADR guidance, explicitly recording new
decisions, distinguishing research from accepted choices, and keeping tasks in
Plane. Reviewed source presents this as an experimental backend and does not
claim a formal backend/default-infrastructure decision. Legacy Go/OpenSpec gates
were not applied. Formatting/type rules enforced by tooling were excluded.

## Optional recommendations

1. **Possible Duplicated Code** — `src/rtc_member.rs:121` and
   `browser-probe/src/rtc_member.rs:62` independently define the same seven atomic
   counters and identical report shape:

   ```rust
   struct PumpStats {
       sent_packets: AtomicU64,
       sent_bytes: AtomicU64,
       received_packets: AtomicU64,
   ```

   The reports duplicate keys such as `"actualSentPackets"` and
   `"droppedQuicBytes"`. Adding/changing a counter requires synchronized edits
   to both platforms and risks inconsistent evidence. Move this small
   platform-independent type/report into the already shared RTC module.

2. **Possible Duplicated Code** — `browser-probe/src/rtc.rs:308` adds `detach()`;
   `Drop` repeats its callback-unbinding list at line 322:

   ```rust
   self.dc.set_onopen(None);
   self.dc.set_onmessage(None);
   self.dc.set_onclose(None);
   ```

   A future callback could be detached on only one cleanup path. Have `Drop`
   call `self.detach()` after `self.retire()` and retain `callbacks.clear()`.

Both recommendations are smell-baseline judgement calls, not acceptance gates.

## Readback evidence

Read global RTK and code-review skills; root AGENTS, README, CONTEXT;
`docs/agents/domain.md`, `issue-tracker.md`; product-direction/scenario entry
points; ADR0004/0005; browser-WebRTC replan. No additional active AGENTS or
coding-standards files were found outside the legacy archive.

Reviewed the complete output of:

```text
rtk proxy git diff f5bae5c727c632faf5cac0b3473b01cbbe938fe4 --
```

Also read all six untracked sources: `src/rtc_transport.rs`, `src/rtc_member.rs`,
`src/rtc_wire.rs`, `browser-probe/src/rtc_member.rs`,
`browser-probe/harness/run-rtc-member.mjs`, `tests/rtc_path_qualification.rs`.
Verified ref/HEAD, empty commit list and WIP status. No source/index/commit
mutation, build, test, tracker access, or network traffic performed.

Result: 0 required findings; 2 optional recommendations.
