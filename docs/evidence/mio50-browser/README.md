# MIOPUNCH-50 evidence

Runtime/API ownership and supported scope are described in
[the browser admission note](../../notes/2026-10-02-browser-manual-admission.md).
The rerunnable entry is `scripts/check_browser_network.py`; use a new output
directory, optionally `--case NAME`, and `--checks` for affected native checks.
No duplicate source archive or frozen executable is required.

- `final-suite/results.json` records the final ordered suite's real commands,
  exits and timings. `browser.log` contains the eleven actual browser scenarios
  and both owned fixture cleanup exits; `scenarios.json` extracts those results.
- `final-suite/*-version.log` records actual Rust, Node, wasm-bindgen and Chromium
  versions. `dependency-tree.log` records the browser WASM dependency route.
- `final-suite/native-*.log` reports affected regressions: 10 proof, 18 gate,
  14 manual admission and 1 RFC vector entries. The manual suite includes two
  inert worker functions and one broker-absent MQTT return; do not count those
  three as real network checks. No slow NAT matrix ran.
- `development/first-red-browser.log` records the prefeature missing browser
  export. `first-green.log` and `priority-green.log` preserve the required real
  startup and subsequent freeze/reopen checks in their actual order.
- `development/gated-data-red.log` records the genuine member data rejection
  after enabling the fork's admission gate while returning plain `Accept`.
  Its `AcceptData` correction is verified by `tls-green.log` and the final suite.

Earlier local environment attempts were not product REDs: Chromium was absent
from the active cache; the binding generator was outside PATH; and the initial
fixture cleanup waited on a pending Tokio stdin read. The first serving fixture
also dropped a reply connection too soon. Those setup/lifetime failures were
corrected before successful admission was claimed. Intermediate forced cleanup
is retained in the development RED; final cleanup has two actual exit-0 receipts.
The earlier owned leftover directory was explicitly removed. No private key,
secret, native SQLite file or browser profile is copied into these artifacts.

The TLS test uses a generated loopback certificate, explicitly trusted by native
and excepted in the isolated Chromium fixture. It proves real HTTPS/WSS plus
end-to-end QUIC/TLS admission and data, not public production availability or
production trust deployment. Browser process restart uses a real disk profile;
the other independent applicants have separate browser contexts and identities.

Independent Standards and Spec reviews are recorded separately in
`standards-review.md`, `spec-review.md`, and their `code-review.md` aggregation
(completed against the implementation commit). Architecture suggestions without a confirmed bug remain parked
under the user's latest instruction.
