## Context

The historical lab runtime has many commands from older P0/P1/P2/XTCP/MNT tracks. Current POC v1 does not want to revive that whole matrix. The only VM work being restored now is the old P0 substrate selftest: running the `core-01..10` NAT profile cases inside the QEMU/netns lab and collecting the existing artifacts.

## Goals / Non-Goals

**Goals:**

- Rename the old generic `selftest` gate to `nat-profile-selftest`.
- Make `nat-profile-selftest` the only required VM lab gate for current validation.
- Preserve the exact `core-01..10` baseline case behavior instead of converting larger historical suites.
- Keep artifacts and failure evidence compatible with the existing guest runtime.

**Non-Goals:**

- Do not revive `xtcp-*`, MNT, POC e2e, TCP Door-2, or remote-broker VM gates as required current validation.
- Do not mutate the `core-01..10` NAT profiles to fit current POC v1 behavior.
- Do not introduce new product pathing logic or Go runtime changes in this change.

## Decisions

- Use `nat-profile-selftest` as the command name because it describes the test content: NAT profile validation, not product e2e validation.
- Remove `selftest` as a current command rather than keeping an alias. Keeping the alias would preserve the ambiguous name and make it easier for future work to treat unrelated selftests as the mainline gate.
- Rename the guest runner from `mlab-selftest` to `mlab-nat-profile-selftest` so host and guest names stay aligned.
- Keep old `xtcp-*`, `poc-e2e-*`, and `mnt*` commands available only as historical/debug commands. They are not converted or run by the current gate script.

## Risks / Trade-offs

- Removing `selftest` can break old local muscle memory. Mitigation: `labctl --help`, `lab/README.md`, and developer guidance point to `nat-profile-selftest`.
- The VM gate can be slow or blocked by host QEMU setup. Mitigation: keep host checks separate and document QEMU prerequisites; failed lab setup is reported separately from host test failure.
- This gate validates the NAT lab substrate, not full POC v1 product e2e behavior. Mitigation: the capability name and spec text state that scope explicitly.
