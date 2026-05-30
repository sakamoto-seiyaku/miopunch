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

## Failure rule

If `join` fails, record `stage`, `reason_code`, `facts`, and `suggestions`.
