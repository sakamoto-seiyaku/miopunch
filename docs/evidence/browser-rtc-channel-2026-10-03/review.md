# Step-4 two-axis review

Fixed point `6cd3ada172683596025766040baa884d03e9124d`; initial snapshot
`207825e08acfea368f83709957f58d5df0de2403`; fixes reviewed at
`e6079ae14878784e5e0091d3287cfc3bb2dc9650`. Independent, read-only
`gpt-6.1-sol/high` agents reviewed Standards and Spec separately. Their follow-up
checks examined the fixes and saved source hashes without rerunning network tests.

## Standards

No documented-standard violations against AGENTS.md, RTK.md, CONTEXT.md or
docs/agents/domain.md. The evidence retains the boundary between experiment
qualification and accepted product/library decisions.

The initial nonblocking **possible Duplicated Code** finding concerned independent
positional binding-context serialization and exporter labels in browser/native.
Those now use shared `attempt_context` and `EXPORTER_LABEL` in
[src/rtc_association.rs](../../../src/rtc_association.rs). Platform-specific
connection exporter calls remain local, as recommended. Recheck found no new
concrete Standards finding in the cancellation code or exercised withheld-reply
fixture.

**0 remaining documented violations; 0 remaining baseline smell findings.**

## Spec

Initial P2: the requirement “close/cancel 唤醒等待者，迟到 onopen 不复活” was
partial because browser work observed only its timeout, while exposed caller
close terminated M without directly retiring RTC.

The browser now prioritizes original-M closure and retires the association;
onopen checks that same M before opening queues. The real second-qualified-M
fixture closes M while browser RTC receive is pending. The saved
[red receipt](caller-close-red.json) waited **16136.4ms**; the corrected
[receipt](channel.json) rejected locally in **2.5ms**, recorded three created/
three closed browser peer connections, detached callbacks, zero native queues,
socket counts **6→6**, and native/relay exit code **0**. Source hashes match
the reviewed files.

**No remaining or new Step-4 Spec findings; no scope creep or independent
incorrect implementation found.** Authorization-revision experiments, path
leases and same-M data-path switching remain later-step work.

Standards: 0 remaining (initial worst: nonblocking duplication judgment).
Spec: 0 remaining (initial worst: P2 public-close cancellation gap).
