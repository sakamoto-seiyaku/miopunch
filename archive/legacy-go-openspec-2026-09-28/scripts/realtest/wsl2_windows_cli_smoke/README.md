# Windows/WSL CLI Smoke

This directory is a CLI-only runbook and script skeleton for bidirectional Windows/WSL join smoke.

Expected artifacts per run:
- CLI stdout/stderr
- `--report` output
- daemon logs
- runtime/state snapshots
- run metadata

Run order:
- Windows `up -> init-network -> invite -> approve -> join`
- WSL `up -> init-network -> invite -> approve -> join`

Use isolated bundle roots per side.
