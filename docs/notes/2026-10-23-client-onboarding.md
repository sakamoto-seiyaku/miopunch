# Public network onboarding without UI

MIOPUNCH-113 implements the non-UI operations in the
[first-release PRD](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6159ec62-a03e-4eb7-9b95-4ddc19ce7a02).
`NativeClient` and `BrowserNetwork` reuse the existing independently identified
nodes, signed management history, admission delivery, resource policy and durable
stores. These operations do not implement the later credential-custody,
management-recovery, guest, service-ACL or per-operation genesis-confirmation
flows. Existing low-level SDK entry points retain their contracts.

## Create and save the original credential

`NativeClient::create[_with_resources]` and `BrowserNetwork.create(configuration)`
create an independent device and genesis credential. Normal creator operations
remain gated by `AwaitingRecoverySave`. Export the recovery card, save it through
the host's private file/print/password-manager facility, then invoke
`acknowledge_recovery_saved` only after the user's explicit saved confirmation.
No additional password or import/scan read-back is required. Ordinary joining
devices neither obtain a root credential nor enter this recovery-card flow.

Native file hosts use `export_recovery_card_to(path, password)`: the private file
must be created and flushed before export is recorded. Failed file saves cannot
enable acknowledgment. Hosts returning a secret directly can use
`export_recovery_card(password)`; browser hosts use the corresponding WASM API.
Export and saved acknowledgment are separate durable operations. Repeating the
acknowledgment does not generate another network or credential.

The version-one recovery encoding is an engineering format, following ADRs
[0008](../adr/0008-genesis-recovery-credential-pins-create.md) and
[0013](../adr/0013-genesis-recovery-card-without-mandatory-password.md):

| Form | Contents and protection |
| --- | --- |
| `miopunch-recovery:1:` | Base64url of the independent 32-byte Ed25519 seed and original 32-byte canonical Create hash. Printable; no extra password. |
| `miopunch-recovery-encrypted:1:` | The same material encrypted with AES-256-GCM, a fresh 16-byte salt and 12-byte nonce, and PBKDF2-HMAC-SHA256 with 600,000 iterations. Version/domain are authenticated. |

Both forms are explicit secret-bearing returns, never diagnostic DTOs. The plain
card grants access to genesis authority material; QR/text/file carriage does not
encrypt it. `RecoveryCard` has no `Debug` or implicit serde export. The bounded
import API can return the public network/Create anchor without revealing the
credential. History, business data and the original device identity are absent;
this card alone is not a complete node migration or history backup.

Browser creator custody follows the same authorization rules. Its credential is
stored in the origin's leased IndexedDB snapshot, subject to browser storage and
page-code trust. Closing a page stops that node; it does not keep receiving
requests in the background or author a membership departure.

## Full invitations and resource choices

`NativeClient::invitation`, `BrowserNetwork.invitation` and their precode variants
produce `ClientInvitation`: the pinned root/Create/contact, actual contact
addresses, optional precode and portable contact descriptors. Version two adds
the complete contact envelope; version-one wrapper decoding and the old raw SDK
invitation APIs remain supported. Text and links carry the same bounded material
as JSON (`miopunch://invite/…`). A future QR host encodes that same value; this
change contains no QR renderer or product UI.

Invitations contain neither the genesis private key nor the long-lived admission
secret. A precode does contain its deliberately shareable, short-lived bearer.
Pass the complete invitation through a trusted channel and keep it out of logs.
Contact hints identify a candidate route; they do not establish membership.

Join with `NativeClient::join[_with_resources]` or
`BrowserNetwork.join(configuration, fullInvitation)`. Offered facilities are
imported before binding transports. Defaults may gain permitted trusted contact
resources, while explicit replacements, disabled origins and disabled uses stay
effective. Browser input vectors supplied explicitly are retained as replacement
choices, including empty vectors. Older saved browser vectors are conservatively
retained as replacements on upgrade. A later reopen does not silently restore
disabled defaults.

MQTT and IRC descriptors carry explicit native/WSS mappings. URL resemblance does
not invent an alias. Native-only signaling descriptors remain native-only
metadata in browser imports. Export includes only permitted contact facilities;
receivers still apply their own policy. A route or relay appearing in an
invitation does not override local denial.

Native imports retain trusted mapping metadata even when the local replacement
or denial prevents its use. The mapping reserves its physical native origin,
including DNS spelling and default-port aliases, against raw signaling fallback.
Retaining it does not authorize a connection: resource selection and native/WSS
or logical-ID denial still apply before binding and export. Saved mapping
conflicts fail the import instead of replacing local metadata.

Raw resource selection is shared by binding, invitation export and native
startup/maintenance discovery. A mapped facility cannot reopen as raw MQTT
discovery through a separate `Lookup` grant after its signaling entry is denied.
Legacy raw discovery continues to require both `Signaling` and `Lookup` on an
unmapped origin; independently permitted other origins remain usable. Mapped
facility tags and existing raw MQTT wire tags keep their distinct contracts.
The existing maintenance owner's live disable retires every spelling of the
selected physical origin, stops subsequent discovery queries to those aliases,
and preserves independently permitted other origins. Saved denial and live
retirement use the same comparison; legacy wire keys stay unchanged.

## Approval, rejection and completion

`NativeClient::requests/approve/reject` and browser
`admission_requests/approve_admission/reject_admission` expose existing durable
manual admission. Browser `admission_requests` retains its ledger-view contract:
hosts show a pending prompt only for entries with neither `approved` nor
`rejected`. Browser hosts explicitly drive `serve_admission(precode)` while
online; the included technical host is not a production background runtime.

| State | Meaning |
| --- | --- |
| `AwaitingRecoverySave` | New creator must finish recovery-card export and saved confirmation. |
| `Pairing` | Independent pre-network receiver has not saved an invitation. |
| `AwaitingApproval` | Ordinary joining request has not obtained verified membership authorization. |
| `Rejected` | This manual request stops automatic retry. It is not a device ban. |
| `PendingMaterials` | Verified authorization is saved, but secret delivery remains incomplete. |
| `Complete` | Current membership authorization and admission secret are verified and saved. |

Connection progress is also reported by the bounded operation and connection
diagnostics. A completed local admission does not claim globally latest history
or that every member is reachable.

The native `request_admission(fullInvitation)` and browser
`request_join(authorizationOnly)` continue the saved attempt. Authorization-only
returns before secret completion and is useful when a host must stop/reopen.
The browser also retains its old raw request and explicit completion APIs.
Retries reuse the request or redemption result rather than issuing another grant
or consuming another use. Closing/disconnecting preserves saved membership.

Rejection is durable for the authenticated `(applicant, request)` pair. It cannot
be revived by approving that old request. The explicit
`new_manual_attempt` operation creates a fresh request while retaining device
identity. Receiving another valid precode is a separate attempt namespace.

If the original delivery node is unavailable, close and reopen the pending native
client with `NativeClient::resume(path, newFullPrecode, bind)` (CLI `resume`), or
reopen the browser with `BrowserNetwork.join(configuration, newFullPrecode)`.
The fresh code belongs to its actual designated administrator and consumes its
own use. Existing still-valid membership is delivered rather than blindly added
again. This does not transfer or take over the original code's redemption ledger.

## Reverse pairing

`NativePairing::start` or `BrowserNetwork.pair` creates/persists an independent
receiving identity before selecting a network. `information` /
`pairing_information` returns private `PairingInfo` text/JSON for trusted transfer
to the administrator's scanner. It includes a random 32-byte capability and the
receiver's actual contact. Treat it as a bearer, not a public node-status value.

`deliver_invitation` uses a dedicated, control-only pairing ALPN on the existing
administrator identity. The supplied root/Create/contact must match that
administrator's own network invitation. Its HMAC binds the exact full-invitation digest, both
authenticated endpoint identities and a TLS exporter. The receiver also checks
that the invitation contact is the authenticated TLS sender. Wrong peers,
substituted capabilities/routes and later replacement material fail. Neither
NodeID/routes alone nor successful pairing release a member/business handle.

The receiver commits the invitation and local resource selection before `SAVE`.
An unsuccessful durable save is not acknowledged. Exact delivery retries are
idempotent; another invitation cannot replace the accepted one. Native
`join` / CLI `finish_pairing`, or browser `finish_pairing`, opens the saved node
with that same device identity. Ordinary approval/precode redemption is still
required. Browser finish waits for its Web Lock to be released before immediate
rebinding, preserving the existing exclusive-identity guarantee.

The native receiver bounds concurrent handshakes to four and serializes only
durable publication. A stalled handshake or delayed peer close cannot occupy the
only receiver or hold the publication lock. A retry after interrupted publication
may reuse existing state only when its device identity, network anchor and exact
saved invitation all match; another target is never overwritten.

Pairing uses the receiving endpoint's available, locally permitted contacts.
Resources disabled at either endpoint remain disabled. A missing usable contact
fails explicitly; scanning route material is not permission to install or enable
an otherwise unavailable carrier. Pairing capabilities live for that receiver
instance; closing/cancelling it retires its listener and any pending exchange.

## Minimal hosts and validation

The JSON-lines CLI extends [the native client host](2026-10-23-native-client.md):

```sh
rtk proxy /path/to/pinned/miopunch-client create /private/new-node [RESOURCES_FILE]
rtk proxy /path/to/pinned/miopunch-client join /private/new-node /private/invitation [RESOURCES_FILE]
rtk proxy /path/to/pinned/miopunch-client pair /private/new-node [RESOURCES_FILE]
rtk proxy /path/to/pinned/miopunch-client resume /private/pending-node /private/new-precode
```

Recovery export takes `path` and optional `password_path`; credentials are never
returned on stdout. `invite` accepts optional `text:true`; `issue_precode` takes
`seconds` and optional `max_uses`. `reject`/`approve` take the public
`applicant`/`request_id`. Pairing export and delivery take private file paths.
All control input remains bounded in both pairing and ordinary host phases.

Browser exports are real WASM methods. The host supplies configuration JSON,
private invitation/card transfer, and explicit confirmations. JavaScript uses
`bigint` for the precode APIs' `u64` values. There is no product page, layout,
visual component, installed shell or browser-specific role restriction here.

`scripts/build_native_client.py` and `scripts/build_browser_client.py` freeze
source/lock/toolchain inputs, use the shared Cargo lease, preserve raw Cargo
counts and selected depfiles, and pin immutable artifacts. Runtime drivers
`check_client_onboarding.py` and `check_browser_onboarding.py` only consume
verified pins. See the [scoped evidence](../evidence/client-onboarding-2026-10-23/README.md)
for actual platforms, positive/failure cases and retained provenance limits.
