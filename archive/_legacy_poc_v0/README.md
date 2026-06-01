# Legacy POC v0 Code Archive

## Purpose

This directory stores the removed legacy POC stack so the active
`poc-v1-01..07` changes can be reorganized against a stable reference without
continuing to treat the old implementation as the design authority.

This archive is not wired into the active root module build.
The removed live paths are kept here for reading and selective salvage only.

## Snapshot Scope

Copied here from the active tree on 2026-05-24:

- `internal/controlplane/`
- `internal/pocstate/`
- `internal/task/`
- `cmd/miopunch/poc_commands.go`

These paths were selected because they currently mix legacy POC control-plane,
state, task orchestration, and CLI entry behavior that the `poc-v1` extraction
is explicitly trying to replace or narrow.

## Not Archived Here

The following areas were intentionally left in place only and were not copied
into this snapshot root because they are still expected to remain reusable as
shells or leaf mechanics for extracted v1:

- `internal/poc/`
- `internal/localapi/`
- `internal/desktopbridge/`
- `internal/punching/`
- `internal/punchwire/`
- `connectivity/`
- `dataplane/`
- `internal/tlsutil/`

The same paths also exist under `live_removed/` as the exact locations removed
from the active source tree when the repo was hard-cut over to rebuild mode.

## Current Status

- The active codebase no longer contains live `internal/controlplane`,
  `internal/pocstate`, `internal/task`, or `internal/pocacceptor` packages.
- Existing callers that still import those paths are now intentional rebuild
  breakpoints.
- Treat this archive as a frozen reading/reference surface while the extracted
  v1 implementation moves into `internal/pocv1/*`.

## Next Step

Rebuild the remaining callers onto `internal/pocv1/*` or other retained leaf
packages, then delete any transitional references that still point at the
removed legacy paths.
