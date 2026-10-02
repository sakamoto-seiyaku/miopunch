# MIOPUNCH-48 evidence

Implementation behavior, decisions and limits: [member data relay note](../../notes/2026-10-02-member-data-relay.md).
Public seam tests: [tests/member_relay.rs](../../../tests/member_relay.rs).
Rerun: `rtk proxy python -B scripts/check_member_relay.py --output NEW_DIR --suite`.
The output path must be new; `--case NAME` runs one named condition.

- [final-suite/results.json](final-suite/results.json): ordered thirteen-case
  acceptance, each exact selector executes one real test; first check precedes
  priority failure. Its affected 47/184-short/type/WASM/format child passed.
  The wrapper then records an IRC harness setup failure (missing owned-server
  environment); this is retained rather than overwritten.
- [signal-isolation/results.json](signal-isolation/results.json): additional
  actual live signaling service while data permission is withdrawn. The original
  business session's 65,536 queued bytes do not produce signal datagrams, live
  signal routes remain, and a fresh restricted attempt fails business release.
- [final-suite/path-maintenance-short/results.json](final-suite/path-maintenance-short/results.json):
  the actual 47 maintenance matrix and affected short regression, owned broker
  lifecycle, library/type/WASM/format receipts. Already verified unchanged slow
  NAT matrices are explicitly skipped.
- [continued-short/results.json](continued-short/results.json): authoritative
  remaining IRC regression with owned usable/limited Ergo servers, default build,
  extended changed-fork formatting and owned cleanup. Only the missing harness
  setup was corrected; already passed acceptance and short checks were not repeated.
  The one existing privileged IRC UDP-cut case remains explicitly ignored;
  unchanged prior 45 evidence is not presented as a new 48 physical-cut result.
- Independent Standards and Spec reports are recorded separately after review;
  reviewers inspect the implementation against the fixed pre-48 baseline.

The acceptance environment is native Linux IPv4 loopback with independently
persisted identities/stores, real signed membership, TLS/SCRAM/QUIC, native TCP
outer links and real controlled UDP backup. Logs contain secret-free result
markers and carrier counters, not fixture private keys or exporter/secret bytes.
Initial failing runs are classified in the implementation note, without claiming
that a fixture with a UDP backup proved sole-path failure. No public Internet,
new NAT topology, browser circuit runtime or cryptographic audit is claimed.
