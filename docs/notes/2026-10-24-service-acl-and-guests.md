# Service ACL and temporary file guests

[MIOPUNCH-115](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/9dbf4901-1310-49d0-bb20-2d5dbb5994ff)
extends the existing NativeClient and independent BrowserNetwork file module.
The [qualification index](../evidence/service-acl-guests-2026-10-24/README.md)
identifies actual source graphs, platforms and retained failures.

Each node declares its own file capability and switch state. The declaration
is signed by that provider, saved and propagated with service control history.
Native provides filesystem receipt; browser capability also checks the actual
File/Blob and OPFS APIs. Opening a host publishes its receiver as disabled.
Explicit enable prepares platform storage and saves the declaration before the
receiver becomes available. Enabling preserves an existing access restriction.
Remote declarations describe the last verified publication, with reachability
and the provider's current execution checked separately.

## One effective policy

The rule for one provider and service has member defaults, guest-sharing
permission, explicit member allows and explicit denies:

```json
{"members":true,"guests":true,"allowed":[],"denied":[]}
```

The default permits ordinary members and administrators after local enable.
An explicit member allow still requires current network membership. A deny
takes precedence within this same rule. A guest additionally needs a valid,
identity-bound invitation for this provider and service; an allow entry alone
does not supply that qualification. The `guests` restriction also applies to
creating and redeeming a share.

Daily administrators edit a provider's rule remotely; a member edits its own
rule locally. Both author the same signed record kind and revision chain with
the node key. Neither operation uses a genesis credential or elevation window.
The expected previous revision is required when a rule already exists. Stale
or branching updates fail. This is a sequential edit contract; concurrent-edit
resolution remains outside this package.

Original p2panda signed headers contain the network anchor, typed action,
management causal heads and nonce. Every successor's management view must
include its service predecessor's management dependencies. New effective heads
also require the author's current authorization episode; retaining an old
administrator snapshot cannot restore lost authority. Reopen preserves already
accepted historical rules. A current authorized successor can carry historical
dependencies in a complete incoming batch. Historical authorization is rebuilt
on the signed predecessor graph; a later StrongRemove correction cannot make
retained service history block receipt of that correction. New service heads
are verified as one batch before policy publication. Independently valid
management is saved and enforced even if accompanying service heads are rejected;
that exchange reports the policy failure after committing management. Native saves original headers in the private
`services.records` file through atomic publication; browser saves them in its
strict IndexedDB snapshot transaction. Authorization becomes effective after
that save. Service synchronization transfers original signed control history,
including its management dependencies, over the existing member connection.
Its success means this exchange was verified and saved locally.

The provider checks the effective rule when receiving an actual file offer.
Applying a denial closes affected file sessions and cancels owned tasks and
partials. The same runtime maintenance checks guest deadlines. Existing file
storage ownership joins cleanup before reporting the terminal result.
The gate can cancel a denied transport before an offer reply; a sender can
therefore observe `failed` or `cancelled`, with no successful receipt. Denial
does not create a receiver task or destination.

## Guest qualification and receipt

An administrator can share a selected node's file service; an ordinary member
can share its own service. Creation requires an explicit future deadline.
There is no selected default duration. The invitation contains a short-lived
bearer, its signed offer, trusted network anchor and contact material. It does
not contain a genesis seed or network admission secret. Share it privately.

The designated issuer verifies that bearer on the authenticated control
connection and saves the first binding to the actual peer identity. Another
identity cannot redeem it. The same identity can continue or reconnect while
the original grant remains valid. The link and qualification retain the exact
creation-time deadline. Granting or restoring an issuing administrator's role
does not revive invitations from an ended authorization episode.

Guest file traffic uses the existing file protocol and connection coordinator.
The gate admits only the authorized file peer pair through that scope. A guest
does not acquire ordinary membership, generic member traffic or service-policy
synchronization authority. Guest status exposes its sharing purpose and time;
this does not promise complete isolation of authorization metadata needed for
verification.

A guest offer reserves one of the existing four task slots and exposes
`awaiting_approval`. It creates no destination or partial file at that stage.
One connection exclusively owns that reservation through asynchronous platform
preparation and I/O. A duplicate cannot share its decision or open another
writer. Preparation refusal ends the owned reservation after any existing
partial has been cleaned, so repeated refusals cannot exhaust the task slots.
An interrupted guest's failed confirmation attempt also joins its retained
partial cleanup. Committed receipts and files survive a later same-ID metadata
mismatch; that new request fails without adopting the old receipt. Disable keeps
the partial discoverable until owned cleanup completes, and cleanup failures
remain visible as `save_failed`.
Approval allows this file's existing bounded stream and platform save. Rejection
ends it without a saved receipt. Confirmation neither extends the invitation
nor changes the ACL. Ordinary authorized members retain automatic receipt.
The sender's `complete` result still requires the actual saved-file receipt.
An already enabled Native receiver adopts verified membership from manual or
precode admission automatically; a newly admitted member can send without a
service synchronization or another enable action.

Expiry ends real access under the provider's local clock. Active exit first
saves the guest's signed end record and stops its local file work, then sends
that record to the issuer and provider. Its result distinguishes local exit
from each peer's durable receipt. Administrator revocation is another signed
end record; a remote provider applies it when it receives and verifies it.
Normal disconnection retains the original qualification and deadline.

## Public operations

The JSON-lines Native host adds these operations; SDK methods use the matching
Rust names listed below. Existing file send/status/cancel operations remain the
business boundary.

| CLI operation | Arguments | SDK / result |
| --- | --- | --- |
| `service_status` | — | `service_status`: local capability/switch, observed signed declarations, effective policies and guest status. |
| `service_rule` | `provider`, `service`, `previous`, `rule` | `set_service_rule`: saved record revision. |
| `service_sync` | peer `address` | `sync_services`: bidirectional verified control history, `saved_locally`. |
| `guest_issue` | provider `address`, `seconds`, private `path` | `issue_guest(provider, expires)`: private complete invitation. CLI saves it instead of printing the bearer. |
| `guest_redeem` | — | `redeem_guest`: verify and save the current invitation's identity-bound qualification. |
| `guest_status` | — | `guest_status`: `GuestPending`, `GuestActive`, `GuestExpired` or `GuestEnded` with scope and deadline. |
| `guest_revoke` | `invitation` record ID | `revoke_guest`: save an authorized end record. Synchronize it to a remote provider. |
| `guest_end` | — | `end_guest`: local end and individual durable-delivery results. |
| `file_approve` / `file_reject` | `task` | `approve_file` / `reject_file`: accept one pending decision. |

Start a Native guest with `miopunch-client join-guest DATA_DIR INVITATION_FILE`.
Reopen its existing identity with the ordinary `open DATA_DIR` operation.
`disable_files` now returns its signed declaration/storage result after physical
receiver cleanup. Await Native `close` before reopening the node directory.

BrowserNetwork adds `join_guest(configuration, invitation)`, `service_status`,
`set_service_rule`, `sync_services`, `issue_guest`, `redeem_guest`, `guest_status`,
`revoke_guest`, `end_guest`, `approve_file` and `reject_file`. Structured values
use the existing JSON-string SDK convention; Unix-second `u64` arguments use
JavaScript BigInt. Background service receipt commits IndexedDB and updates the
effective gate. `refresh_service_state` adopts that verified snapshot for a
subsequent foreground control operation. File access uses the current installed
snapshot. Concurrent snapshot replacement fails its comparison instead of
overwriting another accepted commit.

Await browser `close` to join its service/file owners and release the identity
lease. A page stop remains disconnection. Reopen verifies retained identity,
control history and saved files, with receipt disabled until explicit enable.
Storage clearance and page termination retain their actual platform limits.

The files and service-history additions are native node state that future
whole-node backup/migration integration must retain. This package provides the
file service and its common authorization seam; the other four business modules
and installed-platform lifecycle remain assigned to their own delivery packages.
