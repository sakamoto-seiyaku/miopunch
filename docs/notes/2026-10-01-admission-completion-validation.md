# Resumable admission and completion through another administrator

MIOPUNCH-35 completes the [assigned delivery behavior](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/063223d0-62c8-4d83-9420-02896053f26c)
against parent MIOPUNCH-14 and ADRs0002/0003/0004/0007/0008.
Baseline: `33f4ba24b0fd93cde79bea5cff4993a9634be7bb`.
This records a bounded public entry and verification, without changing the
accepted admission, precode, identity or authorization contracts.

`NativeNetwork::begin_precode_completion` accepts personally trusted new-code
material for the node's same canonical network. For an incomplete joining node,
it durably replaces only the delivery-contact binding, retaining its stable
node key, network anchor, signed history, any saved material and original
admission-attempt nonce. Repeating the same contact is idempotent across reopen.
A new code already has its distinct deterministic redemption namespace in the
shared precode path; its own designated node consumes its own quota. Returning
to the original contact preserves the original attempt rather than silently
charging that original code again. A completed member cannot change to another
contact through this incomplete-admission entry. `begin_rejoin` remains the
separate fresh-approval lifecycle entry for effective loss of membership and
still rejects a currently valid member.

This local binding operation grants no authority and does not approve, consume
or report admission success. The existing formal restricted request verifies
the actual TLS peer against the bound contact and canonical invitation.
Existing approved-result queries and secret release recheck the current causal
validity of the specific grant. The current delivery node must be an actual
administrator, and the secret must match its durably pinned current credential.
The receiving node independently verifies full signed history and its local
current grant, saves authorization, then privately saves/pins the secret before
installing it in the gate or acknowledging receipt. An old response or an old
approval cannot reverse a locally verified effective removal.

A different administrator obtains and verifies existing membership using the
current public history/replica boundaries. The real priority fixture uses
actual authenticated LogSync from the original node before that node goes
permanently offline. The new code consumes one new use and reuses that exact
valid grant without another Add. If the current graph instead records effective
removal, the new code follows normal new-admission rules and signs a different
grant; the original invalid result stays rejected. Neither path takes over the
original code, distributed counter or old authority episode. Losing and later
regaining administrator status still does not revive codes from the old episode.

A small shared private binding writer performs the existing private-file fsync,
atomic rename and parent-directory fsync for both lifecycle operations. Normal
builds otherwise reuse the existing MIOPUNCH-30/31/33/34 delivery and durable
progress implementation. Thirteen explicit test-feature checkpoints reuse the
existing real observer for process exits across material creation, sending,
verification, saving and acknowledgements. Receiver completion remains a
separate local observation from the sender's acknowledgement record: a raw
peer acknowledging twice without saving does not make its node report success.

Reproduction, actual failures, the checkpoint matrix, source manifests, complete
raw results and independent reviews are in
[the evidence directory](../evidence/mio35-delivery/README.md).
The native environment uses real signatures, TLS/QUIC, SQLite and LogSync;
WASM validation is compilation only. Physical power cuts, new NAT probability,
other platform runtime, rotation, migration, arbitrary original-code takeover,
UI, service ACL and TUN remain outside this delivery.

The final full workspace/fault suite passed314/0/4 existing ignored, with actual
MQTT94 and all five test/compile/format commands exiting0. The overall runner
exited1 solely because a case-sensitive absence parser misread Docker's lowercase
`no such volume` after successful owned-resource removal. Original receipts and
runner bytes remain immutable. Only that cleanup helper was corrected and
checked against the exact absent owned container/two volumes; no completed suite
was repeated, and all18 other source/test/config hashes remain unchanged. Current
runner SHA and original-run provenance are recorded separately. Both fresh independent GPT-6.1 Sol HIGH reviews inspected the44 staged paths,
complete evidence and bounded cleanup harness delta. Standards found0required
and one optional P3 checkpoint-type maintenance suggestion; Spec found0required
or optional findings. Their verbatim reports are linked from the evidence README.
No runtime, test or runner changes followed their reviews.
