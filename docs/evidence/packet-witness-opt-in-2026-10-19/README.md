# Explicit packet witness compatibility

[MIOPUNCH-104](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/2ed5ec95-0117-4e33-9f4b-29aba2653680),
2026-10-19. Implementation `7522ee46`, contract clarifications `54a55b83` and
`26a107d1`. Fixed point `d1b873e93797a6bbe228eed108a09d44600f249c`.
See the [contract and work classification](../../notes/2026-10-19-packet-witness-opt-in.md)
and [two-axis review](review.md).

The implementation retains the existing process-single, explicit runtime
`install` / `finish` observer. This ticket changes tests and documentation;
production Rust, default features, permission policy and scheduling are unchanged.
The diagnostic module is compiled into the selected Native, WASM and ordinary
Noq graphs. `doc(hidden)` does not provide compilation isolation. No independent
diagnostic feature was added.

Before the first repository edit, fixed original and fork Noq consumers passed
single/healthy streams, and Iroh consumers passed direct, relay and legacy
flows. Final affected checks passed **28 outer tests**: one integration test
running ten fresh diagnostic child processes, 23 protocol tests and four actual
path-guard cases. The ten children are not counted as ten additional outer tests.
All diagnostic cases retain actual 32768-byte Noq UDP/TLS streams and packet
refusal checks, then close endpoints and rebind owned ports. Recursive and panic
callbacks occur inside actual driver polls. Capacity failures deliberately stop
capture while ordinary data continues; they are expected bounded-loss cases.

Off and finished cases observed zero records/callbacks/copied bytes/digest fields.
The enabled sample observed 184 records/callbacks, 79849 reserved field bytes,
50426 copied bytes in fields, 81 digest fields and 150 Weak atomic checks.
These are observed work units, not CPU cost, total allocations, hash-call counts,
timing equivalence or physical-send proof. Native writer costs and capacity are
source-audited; this ticket does not add a Native writer runtime case. Its metadata
mutex can contend while the connection lock is held. The 512 proof-map threshold
marks qualification loss after insertion; it is not a hard map/heap cap. Queue,
core record/field and file-writing budgets are documented separately.

The complete current Native/Browser SDK flow also passed over owned loopback
TLS/WSS and relay facilities: restricted request reached `AwaitingApproval`,
one approval, both required materials durably saved, both identities reopened,
member `PONG`, then normal actor/browser/container cleanup. Existing verified
M103 raw SDK artifacts were reused because selected inputs did not change.
The SDK run's exact older guard snapshot is preserved alongside the later guard
and a continuity record; only nonselected provenance Markdown changed afterward.
No earlier receipt is relabeled as a new build or execution.

[Validation](validation.json), [measurements](diagnostic-work-measurements.json)
and [selected-input continuity](selected-input-continuity.json) summarize the
receipts. The [raw archive](raw-verification.tar.gz) includes build/run drivers,
contexts, source/lock/artifact preconditions, actual selected-source copies,
depfiles, Cargo logs, initial/final review reports, intermediate/final test
sources and exact guard versions. Every member was read back and hashed against
the [manifest](archive-manifest.json). Fixed original upstream source material
and its provenance remain in the linked
[M103 archive](../path-guard-compatibility-2026-10-19/raw-verification.tar.gz),
whose digest is pinned in validation.

Private SDK receipts, actor/store logs, invitation material, TLS keys/NSS and raw
executables/WASM are excluded from this public archive. The actual generated
private invitation values were scanned against every public file and archive
member; [privacy checks](private-material-checks.json) found no matches.
Raw SDK programs and WASM remain locally pinned. This evidence covers selected
Linux/Chrome controlled graphs; it makes no new public-network, physical-device,
full-provider or full-platform matrix claim.
