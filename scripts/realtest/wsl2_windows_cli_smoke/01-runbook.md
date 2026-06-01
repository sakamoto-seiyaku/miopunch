# Windows/WSL CLI Smoke Runbook

Use the session bundle CLI only. Do not use GUI.

## Windows side

1. Start `miopunch up` from the Windows session bundle with an isolated `--state_path`.
2. Run `init-network`.
3. Run `invite --mode approve --uses 1 --expires 15m`.
4. Run `approve <invite_code>`.
5. Run `join <invite_code>` from the other side.

## WSL side

Repeat the same sequence with the WSL session bundle.

## Required artifacts

- stdout
- stderr
- `--report`
- daemon log
- state snapshot
- run metadata

## Failure rule

If `join` fails, record `stage`, `reason_code`, `facts`, and `suggestions`.

## Executability validation

1. Confirm the Windows and WSL bundle roots are isolated and writable.
2. Start `miopunch up --session` on both sides and capture stdout/stderr plus daemon logs.
3. Run Windows -> WSL and WSL -> Windows with fresh state roots for each direction.
4. Save CLI stdout/stderr, `--report`, daemon logs, runtime/state snapshots, and run metadata for every step.
5. Treat the smoke as executable only after both directions have a complete artifact set, even if one direction fails at `join`.
