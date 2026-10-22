# Native client operation boundary

2026-10-23 — [MIOPUNCH-112](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/484dce1e-445a-4e7b-86c4-1e5ede14e10f).
This implements the accepted non-UI P0/P1 slice of the parent client PRD.

`native_client::NativeClient` owns the existing `NativeNetwork` durable node,
its configured `ResourceEndpoint`, and one bounded incoming dispatcher. It
exposes create/join/open, secret-free status and address, private invitation
delivery, request/approval, precode issuance/count, admission completion,
finite member PING/PONG, and explicit close. It reuses the SDK's admission gate,
signed records, SQLite store, resource permissions and connection coordinator;
it adds no transport or separate authorization rules.

The host must await `close` before reopening its state directory. Incoming
exchanges are limited to 16 handlers and 60 seconds each; closing cancels and
joins these tasks before closing the endpoint and store. An invitation is a
capability: `ClientInvitation::decode` validates its version and contact/precode
binding before `join` creates state. Store and transfer it through a trusted
private channel, separately from diagnostic output.

`request_admission` resumes the durable SDK attempt. `Complete` means the
verified member authorization and current admission secret have both been
saved. `receive_authorization` deliberately stops before completing secret
delivery and exposes `PendingMaterials`; hosts can close/reopen and resume
the same attempt. This explicit split makes interrupted admission testable
without corrupting files or introducing a second state machine.

## Minimal CLI host

`miopunch-client create DATA_DIR`, `open DATA_DIR`, or
`join DATA_DIR PRIVATE_INVITATION_FILE` starts the same public module. The first
JSON line is a `ready` event containing public status and address. Commands are
JSON lines with a numeric `id` and an `op`:

| Operation | Fields | Result |
| --- | --- | --- |
| `status`, `address` | — | Public local view or current contact hints |
| `invite` | `path` | Create a new private invitation file |
| `issue_precode` | `path`, `seconds`, optional `max_uses` | Create private precode invitation |
| `precode_status` | `path` | Verified saved use count |
| `request` | `path`, optional `authorization_only` | Actual saved admission stage |
| `requests` | — | Applicant and durable request identifier |
| `approve` | `applicant`, `request_id` | SDK approval stage |
| `ping` | `address` from the peer's public address output | Actual member PONG and selected path diagnostics |
| `stop` | — | Acknowledged only after owned resources close |

Command responses echo `id` and `ok`; errors use `operation_failed` without
printing capability material or low-level error contents. Malformed commands
return `invalid_command`; each line is limited to 64 KiB. Existing SDK JSON
diagnostic events may interleave with command responses, so callers dispatch
responses by `id` and events by `event`. Contact failure can impose the SDK's
shared per-peer backoff even when the next operation uses another protocol.

## Build and verify

`scripts/build_native_client.py` validates a frozen JSON map of repository
input SHA256 values before Cargo, requires coverage of all local source/assets
and workspace manifests, uses the shared cache lease, and pins only the
selected executable. Linux and Windows-GNU dev/default-feature graphs are
distinct. Windows uses the recorded existing compiler image and stable
image-identified wrappers. Per-run pins live under `.artifacts/runs/` on the
same filesystem as the immutable store; `/tmp` contains receipts referencing
these pins. A cross-device hardlink failure is preserved as a failure, never
converted into another artifact store or a silent full copy.
`--validate-only` checks coverage, hashes and the declared Rust 1.98.1 toolchain
without creating a build receipt or invoking Cargo. The regression suite
`scripts/test_native_client_drivers.py` checks omitted/new source and lock drift,
plus cleanup failure isolation and result publication.

`scripts/check_native_client.py` consumes explicit pins and hashes and never
compiles. By default it uses actual Windows `laptop` SSH control plus local
Linux, swaps creator/joiner and connection directions, checks rejection before
approval, request idempotency, authorization-only interruption/reopen, complete
admission, precode max-one retry counts, final identity/membership reopen, and
member business. `--local-only` is a quick local check; `--failure-only` runs
invalid invitations and malformed/oversized control input plus reopen/cleanup.
The remote adapter propagates the native process exit, verifies the deployed
hash, and checks cleanup against the exact owned executable path. SSH/SCP carry
control, artifacts and invitations; member PING/PONG is generated and exchanged
inside the SDK hosts.

The first real two-device qualification and retained raw receipt locations are
in [the evidence report](../evidence/native-client-2026-10-23/README.md).
The repository had unrelated work in progress at the starting baseline; that
source state is retained in the run's source snapshot and frozen input graph.
The feature commit intentionally does not absorb that unrelated work.
In particular, the build driver relies on the existing working-tree
`scripts/cargo_cache.py` and `scripts/pin_artifact.py` and the qualified SDK
source/lock state. Their exact tested hashes belong to the frozen receipts;
the feature commit alone is not a claim that an otherwise clean checkout of
the starting HEAD contains those prerequisites.

This slice does not establish UI, five full services, browser/Android behavior,
desktop background/autostart, or a new network route. Available VPN/IP paths
are functional evidence only. M111's specific RTC investigation and the older
WAN matrix remain separate.
