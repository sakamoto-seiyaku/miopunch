# Prepare the destination before switching networks

MIOPUNCH-54 delivers the native public SDK `NetworkMigration` in
`src/network_migration.rs`. It uses the existing `NativeNetwork`, admission,
resource-aware iroh endpoint, signed management history and real LogSync entry
points. Product UI and additional platform runtime qualification are not part
of this delivery. The behavior follows MIOPUNCH-14 §11, MIOPUNCH-54, and ADR0006.

## Lifecycle entry

Call `NetworkMigration::prepare(source, trusted_invitation)` once on the device
that confirms migration. The consumed source remains accessible through
`active()` / `active_mut()` during preparation. `target_status()` is a public,
secret-free view; the target context itself is not exposed for ordinary use.
`request_target(address)` performs manual admission; `redeem_target(address,
code)` performs precode admission and completion. `use_new_target_precode`
uses the existing same-network new-contact completion semantics. Retries keep
the target's persisted attempt and stable node key, and do not take over the
original issuer's quota.

`validate_target(address)` requires complete durable admission materials and
fetches actual member data through admission-gated LogSync over a permitted
path. A successful invitation/control exchange cannot set the validation fact.
No new transport, retry scheduler, cryptographic proof or success boolean is
introduced. Target resource configuration initially copies the source's saved
choices (including maintenance budget); it can be changed explicitly with
`configure_target_resources` before switch. Forbidden paths remain forbidden
through the shared configured endpoint assembly.

After validation, `switch(source_session)` durably chooses the target, closes
all existing source sessions and rejects new source protocols independently
of subsequent authorization replay. The target becomes the only normal active
context. A source administrator must have a current source genesis elevation;
ordinary members do not need it. `finish_source_exit(source_session)` then
signs and commits the existing self-removal action. These are separate local
steps, not a cross-network transaction. After a restart an administrator may
need a fresh elevation to complete the pending exit; the active network remains
the destination throughout. `source_history()` exposes the signed departure
and its dependencies for existing offline record forwarding. It does not reopen
source business or wait for every source member to receive the departure.

Reopen with `NetworkMigration::open(original_source_directory)`. Ordinary
`NativeNetwork::open` refuses a managed source or destination directory so a
caller cannot accidentally resume both normal contexts. The same exclusive
single-owner directory contract as the native management shell applies; this
SDK does not add a cross-process locking service. A coordinator represents one
confirmed transition and retains its histories and explicit terminal status.
It does not silently delete a network or move other devices.

## Persistence and interruption decisions

The existing source directory remains intact. Its `migration-target` child is
atomically initialized with the same node signing key, the new invitation,
a separate SQLite store, independent secret files and a managed-context marker.
No source genesis credential, contacts, grants, records or delivery rows are
copied. A published source invitation records confirmation before target
creation, so interruption of creation can be resumed.

Preparation and admission status are reconstructed from the trusted binding,
verified target records and saved secret, using the existing delivery commit
rules. Validation, switch and exit facts use separate no-overwrite file
publications: flush a private temporary file, publish atomically, then flush
the directory. A published switch remains effective in memory even if its
subsequent directory flush reports an error. A crash after committing source
departure but before publishing the exit marker replays the signed departure
and completes the marker without another removal. Missing/rolled-back progress
that conflicts with departure fails closed; it never restores old membership.
Arbitrary rollback of all signed history, private files and every external
replica is not claimed detectable. Physical power-loss behavior is not qualified.

The public phases are Preparing, PendingMaterials, MaterialsSaved, Validated,
Switched, SourceExited, Cancelling and Cancelled. A persisted validation proves
the destination was usable at that time; it is not a guarantee of future
reachability. Failure after a committed switch never silently activates the
source. A later destination revocation can still be reopened as a switched
context and follows normal authorization/rejoin rules.

Closing the coordinator pauses preparation and retains both authorizations.
Explicit `cancel_preparation` records intent, ends the known target membership
using its own existing leave rules, and retains the source as active. If target
approval is still unknown, first resume delivery rather than discard a possible
remote grant. If the target independently appointed this device administrator,
cancellation needs that target's genesis credential. Cancellation can resume
after interruption between its intent, signed removal and terminal marker.
`cancelled_target_history()` supplies its signed departure for forwarding;
closing alone never claims that the remote authorization has been removed.

## Network and log isolation

This selected native integration uses per-network `ManagementStore` databases,
anchor-checked signed headers, and anchor-pinned LogSync manifests/snapshots.
Its author log keys are `(author, log_id)` *inside that network's store and
snapshot*, and normal management actions allocate fresh log IDs with independent
causal heads. It does not use the high-level `LogId::from_topic` global topic
mapping. Thus no shared topic-derived author feed is introduced by retaining
the node key. p2panda protocols remain registered with their network-derived
ALPNs; the native replica ALPN additionally requires the network-scoped secret
proof and pinned manifest. Existing MQTT membership-discovery namespaces use
the target scope, while packet route topics carry protected transport packets,
not shared author histories.

The test publishes records under the same stable key in both networks, with
independent destination permission, then synchronizes the destination to a
third node. That node receives the destination record without the source log
prefix, never receives the source-only record, and rejects that source record
when explicitly injected. Source history also lacks the destination record.

## Reproduction and qualification

Run `rtk proxy python3 scripts/check_network_migration.py --output /tmp/mio54-run`.
The runner creates and removes its own loopback Mosquitto container, executes
the ticket's first real check, then its prioritized failure, then remaining
migration checks and the affected short regressions. It preserves real exits
and raw logs. Unchanged slow NAT suites are excluded under the user's policy.
See `docs/evidence/mio54-migration/README.md` for results and independent reviews.

The migration runtime evidence is native Linux with real Ed25519 signatures,
SQLite, iroh TLS/QUIC, SCRAM admission, LogSync and MQTT. Process fault coverage
uses actual exit77 at eight persisted boundaries, including the window after
the source removal commits but before the exit marker. The worker test's inert
entry point is not counted as another scenario. Earlier browser/Windows/Android
qualification does not automatically qualify this new SDK on those runtimes.
The WASM check is compile-only. No NAT success-rate, WAN, installed-app, device
power-loss, UI, ACL, TUN or transparent cross-network business-session claim is
made. Source sessions terminate at switch.
