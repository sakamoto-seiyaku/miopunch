# Durable management recovery from the original root backup

MIOPUNCH-36 integrates [the assigned recovery behavior](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/322a614c-4d6f-4fac-973b-ef2c77912e18)
at the public native node boundary, against parent MIOPUNCH-14 and
[ADR0007](../adr/0007-signed-management-authority-mapping.md) /
[ADR0008](../adr/0008-genesis-recovery-credential-pins-create.md).
Fixed baseline: `f691d2b4c2c0f8188d9e9418ef691c21978a806c`.

`NativeNetwork::recover` prepares a new independently identified device from
the trusted original root private key **and** canonical Create hash. It saves
that credential, network anchor, chosen contact and stable recovery attempt in
an atomically published private directory. It starts with no history, admission
secret or ordinary admission request. Reopening preserves this incomplete
state; it does not mistake the presence of a root key for administrator membership.
Ordinary approval/precode completion remains a separate lifecycle. An incomplete
root-recovery directory cannot switch into ordinary rejoin through `begin_rejoin`.
After completed root recovery and effective membership loss, the same device
can explicitly choose fresh ordinary approval/invitation. Historical completion
is checked against its actual signed root proof and applied root grant, with
saved/pinned material or the durable completed stage. This permits a new
request, never current membership or revival of the invalid old grant. Sealing
that historical stage precedes the existing fsynced atomic join-binding
publication. That single publication selects the ordinary flow; reopen validates
the retained root audit material and activates only the join binding. Device
key, root credential, anchor, history and saved material are retained. Actual
exits before and after publication verify both restart sides.

The formal resumable exchange has its own restricted ALPN, advertised by the
default native endpoint and classified by the existing admission gate. It
reuses MIOPUNCH-25's selected Ed25519 statement, TLS exporter binding and bounded
session guard. The statement binds the pinned root/Create, actual authenticated
recovering and contact identities, and this TLS session. No new crypto selection
or research proposal enters the implementation. Every retry verifies a fresh
live proof; a previously saved proof cannot authorize another session.

An ordinary member can run `serve_management_recovery` when its own membership
is verified and its private current secret matches the durable credential pin.
It verifies the real root signature before sending any history or secret.
The local proof stage retains the actual signed statement and signature, and
reads verify that signed evidence against the original anchor and target. It is
an audit/restart stage, never a substitute for the next live proof or a membership
authorization. The ordinary member receives no root private key and signs no
administrator approval.

After root verification, the member forwards its bounded applied signed
management history (at most4096 records and16MiB). The recovering node checks
the exact canonical Create, signatures, causal authorization and the contact's
membership, then durably merges the history into its own graph. It keeps known
local records and decisions instead of replacing its view with an older response.
The snapshot describes this source's applied history, without claiming global
latest state or recovering data that no node retained.

The new device signs its own root-authorized Add Manage record, targeting its
independent TLS node identity. A fresh random root log starts at sequence zero,
with no old-device backlink and dependencies on the verified graph heads. The
relay checks the root signature, target, exact current dependencies, fresh log
and effective administrator authorization. It atomically saves that exact
signed record with the target/attempt result before applying or acknowledging it.
The receiving node also atomically saves its grant/result before sending.
Reopen applies a committed pending record, and retry reuses the same bytes.

An applied sequence-zero grant is not preflighted again as new authoring: retry
checks its exact saved bytes and **current** administrator authorization. New
grants retain the original append-only log checks. Demotion/removal invalidates
an old result; owning the root backup does not turn that old result into a new
grant. A separate fresh root-authorized operation is needed to change that
authorization. A grant for a substituted identity, an old root log, stale graph
heads, or another Create cannot complete this exchange.

Current-secret delivery follows a separate AUTH acknowledgement. The provider
rechecks the current grant and durable secret pin; the receiver rechecks its
current grant and uses the existing private fsync/publication/directory-fsync
and database-pinning writer before installing the secret in the gate or sending
SAVE. `recovery_progress` derives local Complete from that valid signed grant
and the locally saved/pinned secret. The provider's Complete records the
authenticated SAVE acknowledgement. Neither transport send nor the provider's
receipt claims physical durability on another machine or global convergence.

Recovery preserves the original network, stable new device identity and old
administrator qualifications. It does not automatically elevate a management
session: high-privilege operations still require the explicit temporary genesis
window delivered in MIOPUNCH-32. It does not rotate secrets, migrate networks,
revoke old administrators, or add UI, service ACL, TUN or new transport features.

The first real check, failure-first observations, sixteen root-delivery exits,
two ordinary-flow switch exits, signed adversarial cases, affected regressions, final source
manifests and independent reviews are documented in
[the evidence directory](../evidence/mio36-recovery/README.md).
Native observations use real Ed25519, SQLite, iroh TLS/QUIC and actual LogSync.
WASM checking establishes compilation only. Physical power cuts, other-platform
runtime and new NAT probability are outside these observations.

Independent final Standards and Spec reviews report no required findings. The
Spec review closes the completed-device rejoin defect; the sole optional P3
suggestion concerns duplicated test fixture setup. Final evidence remains
116 passing affected tests, including36 recovery cases and18 actual process-exit
boundaries, with all five checks and the wrapper exiting0. The earlier full
347-pass generation is retained as historical evidence before that repair.
