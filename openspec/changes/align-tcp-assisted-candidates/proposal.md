## Why

MNT-01 showed TCP private listen addresses are currently exchanged as `tcp_direct_addrs`, causing tests and diagnostics to treat assisted/private targets as direct path coverage. TCP needs the same direct-vs-assisted separation that UDP already has.

## What Changes

- Add `tcp_assisted_addrs` semantics for TCP private/local assisted punching inputs.
- Keep `tcp_direct_addrs` limited to true direct candidates.
- Update decision and attempt behavior so direct paths do not consume assisted addresses.
- Allow bounded assisted-only TCP punching fallback when STUN evidence is insufficient but assisted targets exist.
- Update F-002 MNT-01 cases so TCP fallback success is not mislabeled as direct coverage.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-tcp-p2p-v0`: TCP candidate gathering, decision, attempt, diagnostics, and MNT-01 test expectations distinguish direct and assisted addresses.

## Impact

- Affected code:
  - `connectivity` gather/attempt, `internal/wire`, `internal/punchdecision`, MNT-01 case definitions and expectations.
- Public compatibility:
  - No old-node compatibility is required for this round.
- Validation:
  - TCP assisted/fallback cases must report `punching_tcp4`, not `direct_tcp4`.
