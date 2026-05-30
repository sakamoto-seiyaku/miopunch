## 1. Change Setup

- [x] 1.1 Create the Windows/WSL CLI smoke OpenSpec artifacts and lock the bidirectional positive-path scope.
- [x] 1.2 Add a short `docs/notes` runbook that describes the CLI-only test order, required artifacts, and failure evidence.

## 2. Smoke Execution

- [x] 2.1 Reuse the existing session bundle layout and `labctl`/smoke entry model for CLI-only validation.
- [x] 2.2 Define the Windows and WSL isolated bundle roots, state paths, and log locations for the run.
- [x] 2.3 Specify the exact positive-path command sequence for both directions: `up -> init-network -> invite -> approve -> join`.

## 3. Diagnostics and Validation

- [x] 3.1 Require CLI stdout/stderr, `--report`, daemon logs, and runtime/state snapshots for each side.
- [x] 3.2 Ensure join failures record stage, `reason_code`, `facts`, and `suggestions` so they can be debugged without GUI.
- [ ] 3.3 Add the focused validation steps used to confirm the smoke is executable in the real Windows/WSL environment.
