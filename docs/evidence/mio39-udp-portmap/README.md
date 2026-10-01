# MIOPUNCH-39 evidence

Read [the behavior and limits](../../notes/2026-10-01-udp-portmap-validation.md)
and the current Plane ticket for scope. Reproduction uses
`scripts/check_udp_portmap.py --case CASE --output NEW_DIRECTORY`.
Every final case used a real NAT-PMP client, an owned controlled gateway
service, actual Linux DNAT/FORWARD, original endpoint TLS/QUIC and formal
NativeNetwork admission/membership. No probability matrix was run.

## Real checks, in required order

| Case | Actual result and evidence |
| --- | --- |
| First positive | [positive-green/results.json](positive-green/results.json): PASS; mapped actual socket `43400` as `100.64.39.1:41039`, durable manual admission and authorized streams, then owned release. [Gateway wire](positive-green/gateway.jsonl), [pcap](positive-green/gateway.pcap). |
| Immediate priority failure | [expired/results.json](expired/results.json): PASS; administratively removed mapping and failed renewal, old candidate withdrawn, fresh connection unavailable while node remained running. |
| Port change | [changed/results.json](changed/results.json): PASS; replace `41039` with `42039`, withdraw old candidate, fresh member communication on new mapping. |
| Gateway state reset | [restart/results.json](restart/results.json): PASS for state/epoch reset only; superseded for actual process-restart claim by restart-process below. |
| Actual process restart | [restart-process/results.json](restart-process/results.json): PASS; gateway PIDs `56` and `95` in [wire log](restart-process/gateway.jsonl), volatile mapping lost, renewed replacement address and fresh member communication. |
| Successful renewal | [renewed-green/results.json](renewed-green/results.json): PASS; two successful renewals beyond the initial six-second lease, candidate retained and fresh member communication. |
| Protocol disable | [disabled-green/results.json](disabled-green/results.json): PASS; native Disabled sends no gateway request; [pcap](disabled-green/gateway.pcap) contains zero packets. |
| Failure plus another path | [expired-fallback/results.json](expired-fallback/results.json): PASS; removal/failed renewal/withdrawal/fresh failure, then another permitted LAN path carries member traffic. |

Close invariants preserve the foreign owner's permanent `49039` rule and
remove only the node's known mapping. Final rule snapshots and wire deletion
requests are in each active case. For the final renewal, disable, actual
restart and fallback cases, `disabled-protocol-packets.txt` is empty after
captured traffic is filtered for PCP/UPnP packets. NAT-PMP only is the
validated native protocol.

The six-second lease is fixture configuration, not a product lease promise.
Silent deletion detection is bounded by the outstanding lease/renewal cycle,
not instantaneous notice. All native resources/relay were explicitly absent
for the mapped-node checks. The fallback uses a separately permitted direct
LAN route after remote failure, not a disabled relay.

## Failures retained accurately

- [red/serve.log](red/serve.log): genuine first baseline RED; worker exited101
  after eight seconds without a mapping candidate. No gateway request occurred.
- [positive/connect.log](positive/connect.log) and
  [fixture-diagnosis/connect.log](fixture-diagnosis/connect.log): real fixture
  failures. Gateway mapping and TLS worked; the test server closed before the
  applicant read its final DONE acknowledgment. The targeted diagnosis and
  bidirectional [pcap](fixture-diagnosis/gateway.pcap) distinguish this from
  a transport failure. The fixture now waits for the applicant to close.
- `renewed/build.log` and `disabled/build.log`: build-only failures before
  runtime, due to using a node after the consuming backup API. Corrected in
  the test; not counted as behavioral REDs or completed gateway checks.
- `checks/modified-fork-format.log`: unscoped rustfmt checked untouched
  upstream `endpoint/connection.rs` formatting. That incidental formatting
  was restored; the modified-file scoped check passed. No unrelated upstream
  formatting change is included.

## Affected checks

[checks/results.json](checks/results.json) retains each real command exit and
elapsed time. Workspace and browser WASM typechecks passed; WASM is compile
only. The final affected short checks passed:

- resource permissions/configuration:26;
- UDP observations and candidate changes:6 (affected socket surface, so run);
- formal manual admission positive:1;
- native identity/store/restore:5;
- new mapping consent/default/backup/update tests:2, plus an inert lab worker
  entry in [checks/portmap-api.log](checks/portmap-api.log).

This is40 meaningful ordinary checks and the separately exercised real
controlled gateway cases, not a full-workspace or NAT success-rate claim.
Workspace formatting and scoped modified-fork formatting passed. The user
explicitly permits skipping already verified unchanged slow NAT checks;
legacy traversal/selector/multi-socket algorithms and their slow matrices
were not modified or rerun. The p2panda integration was compiled through
workspace typechecking; no unrelated full suite is claimed.

Owned test containers, processes and namespaces are removed by the runner.
Private node stores live only in the container's temporary directory and are
removed before it exits. No private test keys, admission secrets or databases
are committed. Only this ticket's rebuilt, unused executables were removed
to relieve disk pressure; earlier evidence and unrelated user work remain.

## Review-discovered ownership edge

The first independent Spec review found one required P2: allocation was
acknowledged before the separate public-address query, but cleanup ownership
was saved only after both replies. The actual
[allocation-timeout RED](allocation-timeout-red/results.json) confirmed an
owned rule remained after close.
[allocation-timeout GREEN](allocation-timeout-green/results.json) and
[allocation-cancel GREEN](allocation-cancel-green/results.json) confirm
nonblocking cleanup of acknowledged allocations on query failure/cancellation,
with foreign ownership preserved and close acknowledged after cancellation.

A follow-up [renewal RED](renewal-address-failure-red/results.json) exposed
an older candidate remaining after explicit allocation cleanup. The
[renewal GREEN](renewal-address-failure-green/results.json) confirms prompt
withdrawal plus fresh-connection failure. A failed renewal now conservatively
withdraws/releases the older owned mapping, rather than waiting for expiry.

The final genuinely affected checks are retained under
[post-review/results.json](post-review/results.json). Earlier40 ordinary
checks and the unchanged successful port-change/process-restart/disable
paths retain their already passed evidence; no slow NAT matrix was repeated.
Independent final [Standards](standards-review.md) and [Spec](spec-review.md)
reports each have0 required findings; Standards retains1 optional duplication
suggestion. The [two-axis aggregate](code-review.md) records the review scope.
No source edit followed these final reassessments.

