# Browser control replicas and authorized management

MIOPUNCH-51 extends the browser SDK from [manual admission](2026-10-02-browser-manual-admission.md)
to the behavior in [its ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/03a59f75-7575-4ba0-bd1a-f3885a58cd84).
The parent specification's previously agreed client/SDK, real node connection,
and storage-fault boundaries remain the test seams. Product UI remains deferred.

## Shared behavior and platform adaptation

`BrowserNetwork` saves original signed headers **and bodies**, retains causal
history, receives real p2panda LogSync operations and provides applied snapshots
to other qualified members. `management_auth`, `management_record`,
`management_action`, `precode_policy`, `admission_request_id`, `replica_snapshot`
and the existing SCRAM implementation are compiled by both platforms. The
browser uses the existing p2panda framing codec. There is no alternate signature,
password, membership or strong-removal policy, and no remote native execution
of the browser's management commands.

The browser adapter supplies IndexedDB commits, page-local management sessions,
foreground connection handling and the native admission/replica wire formats.
Native keeps SQLite's transactional precode predicates in addition to the shared
new-redemption preflight; browser commits the count, authenticated request and
signed grant in one strict IndexedDB snapshot replacement. Existing redemptions
revalidate their original membership episode instead of consuming another use.
A fresh code uses the shared token-derived request namespace, preserving the
original durable device attempt. Another administrator's new code consumes only
that code and reuses an existing valid grant. Neither platform takes over the
old code.

The only p2panda-sync upstream adaptation disables its normal p2panda-store
default features, which otherwise force SQLite/SQLx and native Tokio networking
into WASM. All LogSync Rust sources are unchanged. See
[the dependency patch note](../../vendor/p2panda-sync/MIOPUNCH.md).

## Durable replica and completion boundary

A complete signed incoming record passes the shared verifier before storage.
Missing causal/log predecessors remain durable pending records, excluded from
member views and serving snapshots. New dependencies drain that pending set;
duplicate hashes do not add another record. Missing bodies, invalid signatures
or unauthorized actions cannot become applied state. Admission responses merge
with retained history rather than discarding locally authored records.

Each replacement uses the existing strict IndexedDB transaction and previous
snapshot comparison. Memory and the live gate change only after `oncomplete`.
An abort cannot send a saved receipt. A source-scoped result reports its source,
manifest IDs, transport completion and whether those IDs were saved and applied;
it never claims global freshness. The receiver waits for the provider to
acknowledge its durable receipt before closing. The native member-replica path
now uses the acknowledgement already present on the recovery-replica path;
otherwise immediate browser connection closure could discard the receipt before
the provider consumed it. Run matching builds on both peers: compatibility with
older application-control wire implementations is not claimed. iroh's external
connection API and p2panda LogSync messages are unchanged.

The current bounded adapter retains up to 4,096 records and 16 MiB of record
bytes, with an 80 MiB serialized-snapshot limit; it fails explicitly at the
limit and does not silently prune history. This is a qualification/resource
bound, not a history-compaction design. Pending records survive reload and
cannot be advertised as applied heights.

## Management and lifecycle

The SDK exposes `begin_management_session`, `elevate_genesis`, `manage`, `leave`,
`exit_genesis_elevation` and `end_management_session`. Session handles are fresh
per session; the root seed must match both the network root and canonical Create.
The caller chooses a finite temporary window. Root material lives only in that
page's active management session, never in IndexedDB or replica traffic. Expiry,
explicit exit, session replacement/end, disconnect or page destruction removes
the available elevation. The same shared command classification and signed
record verifier enforce daily versus genesis privileges, including administrator
self-removal. Demotion retains membership; removal closes qualified connections
and rejects new ones. `Removed` is distinct from saved admission completion.
Closed connections do not revive after later causal correction.

`open_member_connection` returns a controlled member connection whose exchanges
check the current durable state and live gate. `sync_replica(target)` and
`serve_replica()` qualify both identities with the shared TLS-bound SCRAM proof
and current membership. `ingest_management_record` is the signed-record input
seam; `control_state` and `control_record` expose the applied view and complete
original records. Neither exposes private keys, admission secrets or code tokens.

A browser administrator can use `manual_invitation`, `admission_requests`,
`approve_admission`, `issue_precode`, `revoke_precode`, `precode_usage` and
`serve_admission(precode_mode)`. The foreground owner drives one bounded request
at a time, as with the native explicit manual/precode serving methods. This is
not a background resident daemon or a new product interface. The public request
methods retain the same double-material completion contract as MIOPUNCH-50.
`delivery_confirmed` is a sender receipt, not independent proof that a remote
browser saved correctly.

The existing origin-wide Web Lock excludes competing pages; ownership transfer
restores the same key and request. Code counts/results survive normal reload.
Ledger inconsistency or malformed saved state fails closed. Clearing site data
requires explicit fresh admission, and previously held handles cannot continue
using cleared material. There is no browser backup-import API or promise to
detect an externally substituted, internally consistent old profile snapshot;
normal reload and explicit corrupt-state failure are the qualified cases.

## Page trust and sensitive material

The static origin distributes the trusted JavaScript/WASM client. It does not
hold node secrets, approve members or proxy management. Browser storage and
any entered genesis seed are accessible to trusted code running in that origin;
this does not protect against a malicious client update, origin compromise or
browser-profile theft. Deployments must trust the delivered client. Callers must
not retain/log genesis input or bearer precodes. The SDK does not claim guaranteed
JavaScript-memory erasure. Precode tokens remain in private durable request/code
state for retries and are excluded from shared control history.

## Reproduce and qualification

```sh
rtk proxy python -B scripts/check_browser_control.py --output /tmp/mio51-check --checks
```

`--case` runs one incremental case. The runner uses the existing Rust/WASM and
wasm-bindgen 0.2.129 tools, pinned Playwright Core 1.63.0 and installed Chromium.
All control cases use real self-hosted HTTPS/WSS relay access and end-to-end
QUIC/TLS, signatures and IndexedDB. The static test page is a localhost secure
context. Native explicitly trusts the generated test CA and isolated Chromium
allows that test certificate. No public deployment, other browser/OS, direct
UDP/WebRTC path or permanent page residence is claimed.

The [evidence index](../evidence/mio51-browser-control/README.md) records the
actual commands, results, failure classifications and independent review. The
user's standing instruction excludes already verified, unchanged slow NAT
matrices. This ticket does not implement browser network migration, secret
rotation, native maintenance scheduling, service ACLs or a production UI.
