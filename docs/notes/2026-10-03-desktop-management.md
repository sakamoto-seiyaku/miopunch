# Windows and Linux management through the native CLI

MIOPUNCH-52 extends `miopunch-native-probe` with a local JSON-lines management
session over the existing `NativeNetwork` API. Creation, manual admission,
precode redemption, management, replica synchronization, resources, and
backup/restore use the shared signed-record, admission-gate and SQLite paths.
There is no desktop UI, tray, installer, service ACL or TUN/proxy product here.

## Entry points

Build with `cargo build --locked --bin miopunch-native-probe`. The same binary
name with `.exe` is the Windows entry. Existing identity/raw/wrapped/TCP probe
and `network create|status|resources|resources-import|resources-update|run`
commands retain their behavior.

```
miopunch-native-probe network create DATA_DIR
miopunch-native-probe network join DATA_DIR INVITATION_FILE
miopunch-native-probe network session DATA_DIR [BIND_IPV4]
miopunch-native-probe network backup SOURCE_DIR NEW_BACKUP_DIR
miopunch-native-probe network restore BACKUP_DIR NEW_DATA_DIR
```

The optional bind address defaults to loopback for this test shell. Use a
suitable local interface for a real LAN peer. A session opens the configured
endpoint, emits a `ready` event with its public identity/address, and accepts
one JSON request per stdin line. Responses carry the caller's numeric `id`,
`ok`, and either `result` or a secret-free error code. Incoming protocol
failures have separate `incoming_failed` events. `stop` or stdin EOF disconnects;
it does not leave the network. Only one shell should own a data directory.

For example, the administrator writes a trusted manual invitation:

```json
{"id":1,"op":"invite","path":"invite.json"}
```

Pass that file through a trusted channel. On the other device, `network join`
initializes an independent identity using the file, and `network session`
opens that applicant. Its request is:

```json
{"id":1,"op":"request","path":"invite.json"}
```

It initially returns `AwaitingApproval`. The administrator's `requests`
operation lists authenticated `applicant`/`request_id` pairs. An `approve`
request supplies one such pair; the applicant repeats `request` and only
returns `Complete` after both materials are verified and saved. Neither
side has to enter a topic or select a hole-punch sequence.

The session operations are:

| Operation | Inputs and result |
| --- | --- |
| `status` | Public identity, network anchor, effective members/roles, record count, local admission stage. |
| `invite` | `path`: create a manual invitation file. |
| `issue_precode` | `path`, `seconds`, `max_uses` (integer or null): create an expiring invitation capability file. |
| `precode_status` | `path`: saved uses and `count_trusted`; a false value is not remaining quota. |
| `revoke_precode` | `path`: revoke a code at its designated administrator. |
| `refresh_invitation` | `path`, `destination`: write current contact hints for the same invitation/code; never renew its authority, expiry or allowance. |
| `request` | `path`: manual request, precode redemption, or retry. A new code uses the core's same-network completion path. |
| `requests`, `approve` | List authenticated requests; approve the supplied `applicant` and `request_id`. |
| `manage` | `action` (`add_member`, `remove_member`, `appoint_administrator`, `demote_administrator`) and public `target` key. |
| `elevate` | `path` to the 64-byte saved genesis credential, `seconds` for a finite device/session-local window. |
| `exit_elevation` | End the window; restarting the shell also discards elevation. |
| `sync` | Public `address` from a peer's ready event; real LogSync, source-scoped verified completion. |
| `ping` | Public `address`: admission-gated real TLS/QUIC PING/PONG test traffic. |
| `resources`, `save_resources` | Read a `config` or save one through core validation; saved changes take effect on the next endpoint start. |
| `stop` | Close the session and endpoint; preserve membership and durable state. |

`request` also accepts `pause_before_save: true` for the explicit failure
fixture: receive and save authorization, then drop the material-delivery
handle and close the connection before secret completion. The local result
remains `PendingMaterials`. Reopening and repeating the same request resumes
its original durable attempt. The provider can refresh the invitation's
address after a port change without changing that attempt or consuming again.

Error results distinguish `permission_denied`, `elevation_required`, invalid
genesis/session/window, `admission_rejected`, `admission_paused`, incomplete
delivery, and `temporarily_unreachable`. A failed admission exchange does not
reveal whether a remote code is valid; the issuer's local count status is the
place to inspect its own ledger. Status remains independent of reachability.
Errors never echo invitation contents, genesis keys or admission secrets.

Invitation files contain the trusted canonical invitation, contact hints and,
for precodes, the bearer capability. They contain no network admission secret
or genesis key. Creation refuses to overwrite an existing file; Unix files
use mode 0600. Windows uses the account's inherited ACL, so choose an
account-private directory. The controlled runner creates a mode-0700 parent
for its disposable node material. Do not publish the runtime data directories
as evidence; the runner's logs and result JSON are sufficient.

## Minimal integration choices

The CLI needs to serve manual and preauthorized requests at the same time.
It negotiates `miopunch/precode-request/1` for the latter, sharing the existing
restricted `AdmissionRequest` gate and the unchanged precode exchange. The
original `miopunch/admission-request/1` and explicit core manual/precode methods
remain available for existing fixtures. This is an additional protocol label,
not a new authority or cryptographic exchange. Other client shells must opt
into that label to use these CLI precode files; arbitrary older application
wire compatibility and browser consumption of the CLI envelope are not claimed.
The iroh Endpoint/Connection interface and p2panda LogSync messages are unchanged.

`precode_usage_status` exposes the existing local stopped/ledger-consistency
predicate as a secret-free public result. Supported restore deliberately makes
old counts untrusted and stops new redemption, even with apparently unused
allowance; issuing a fresh code is separate. An already granted membership is
not revoked by that status.

The Windows execution found a real Unix-only persistence assumption:
`File::open(directory).sync_all()` failed with access denied. The small
`native_files` adapter keeps Unix fsync/hard-link/rename behavior and uses
same-volume `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` on Windows. New
publication omits replacement permission, while file replacement explicitly
requests it. File data still uses `File::sync_all`, and SQLite retains WAL with
FULL synchronous commits. Windows does not run the Unix directory-fsync call.
This follows the documented [move semantics](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
and [file flushing requirement](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers);
it does not claim physical-power-loss coverage on an untested filesystem.

The locked iroh and iroh-relay packages also emitted unused `cdylib` artifacts
whose debug Windows export tables exceeded the PE ordinal limit. Their
normalized manifests now build the Rust library only. The relay package is
vendored for this packaging change with all upstream Rust source unchanged;
see `vendor/iroh-relay/MIOPUNCH.md` and `vendor/iroh/UPSTREAM.md`. miopunch does
not consume their C ABI. No networking algorithm changed.

## Reproduction and runtime boundary

From a Linux host with Docker and the repository Rust toolchain:

```
rtk proxy python3 scripts/check_desktop_windows.py --output /tmp/desktop-check-unique
```

The runner builds an isolated Debian bookworm MinGW compiler image and Debian
trixie Wine 10 image, builds both CLI artifacts, executes the Windows binary
against Linux, then removes its owned containers. It records every command's
actual exit and the tool versions. Images remain as reusable build cache.
`--case first|restart|management|precode|resources` selects a narrow rerun.
The lower-level `check_desktop_management.py` accepts existing binaries and a
Wine runner; without `--windows-runner` it explicitly records Linux-only
checks and cannot establish Windows evidence.

The first runtime check is a Windows-to-Linux raw QUIC round trip, followed
by creation/manual joining in both directions. Pending-material restart comes
next. Later slices cover daily/genesis management with remote LogSync,
precode counts and restore, and resource restrictions with actual failed
contact/backoff/recovery. Real Ed25519, Windows random-number APIs, TLS/QUIC,
SQLite and private-file operations run in these processes. No success stub or
replacement crypto DLL is used. Wine 8 could not load the Rust runtime's
required `bcryptprimitives.dll`; Wine 10 supplies that API.

Windows evidence means the x86_64 Windows PE artifact executed under Wine 10
on Linux with controlled loopback networking and the host filesystem. It is
not a native Windows kernel/NTFS/ACL certification. Native Windows host
behavior, other architectures, physical power loss, public-service uptime,
new NAT success rates, installers and platform UI remain unqualified here.
The ticket explicitly permits an equivalent controlled execution environment;
no cross-compilation-only result is counted as runtime acceptance.

See [the evidence index](../evidence/mio52-desktop/README.md) for actual results,
first failures, affected regressions and independent review.
