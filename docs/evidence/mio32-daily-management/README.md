# MIOPUNCH-32 evidence

The implementation mapping, public testing seam, observed failures and limits
are in [the validation note](../../notes/2026-10-01-daily-management-validation.md).
The fixed baseline is `8ee52120b6499b5ff70142aba571e2bc43c7ce2d`.

Run from the repository root with the locked Rust toolchain and Docker:

```sh
rtk proxy cargo test --locked --features replica-test-hooks --test daily_management -- --test-threads=1 --nocapture
rtk proxy python3 scripts/check_daily_management.py --output /tmp/mio32-final-new-generation
```

The runner creates an owned broker on a dynamically allocated loopback port,
runs the full workspace with real MQTT and replica fault checks, then WASM,
default/fault-build typechecking and formatting. It records exact commands,
clocks, exits, source hashes and broker cleanup. It refuses existing output
directories and does not overwrite previous evidence or rerun NAT lab matrices.
Network/socket and Docker access must be allowed in the execution environment.

Preserved incremental logs include the first missing-API RED, the socket-binding
environment failure, the first actual pass, the priority window failure checks,
demotion's missing-variant RED and pass, unauthorized raw-record receipt, and
both actual causal wire orders. All 13 raw logs are preserved losslessly as
`.log.gz`; [log-integrity.json](log-integrity.json) records original, compressed
and roundtrip SHA-256 values. Read them with Python's `gzip.open` or `gzip -cd`.

[Final generation 1](final-generation1/receipt.json) completed all five commands
with exit 0: **274 passed, 0 failed, 4 existing ignored**, with real MQTT capture
of 92 publishes. The ten source/harness hashes stayed unchanged. The exact
owned broker's removal was independently
[read back](final-cleanup-readback.json). The
[short-regression receipt](short-regression-receipt.json) records the actual
19/0 tool result and distinguishes its summary from a raw log.

Final independent [Standards](final-standards-review.md) review found **0 required,
1 optional** finding: duplicate access lookup as a maintenance suggestion.
[Spec](final-spec-review.md) review found **0 required, 0 optional** findings.
Both reviewers used fresh GPT-6.1 Sol HIGH contexts, checked the exact 26-file
staged snapshot, source/protected hashes and compressed logs, and made no runtime
edits or repeated test runs. The final publication adds their two verbatim reports
and the observed review results; source and harness remain unchanged.
