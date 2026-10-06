# Unified admission and Replica purposes

MIOPUNCH-83 migrates manual admission, precode redemption and normal Replica
to the common protected session coordinator specified in ADR0009.

Ordinary member business remains `miopunch/session/member/1`, purpose1.
Manual requests use `miopunch/session/admission/1` and precode requests use
`miopunch/session/precode/1`; both carry restricted purpose2 in the strict
MUC1 version1 envelope. Their distinct ALPNs select the existing handlers.
Normal Replica uses `miopunch/session/replica/1`, purpose3. Migrated entry
points do not negotiate old ALPNs or legacy capability versions. Explicit
recovery remains separately authorized and cannot downgrade normal Replica.

Restricted admission checks the actual invitation/contact or administrator
scope and current durable store. Both permitted data masks are zero; the
common dialogue creates no ordinary handle, member proof or data path.
Existing authorization and secret delivery handlers still verify, save and
acknowledge admission material. Approval alone remains PendingMaterials.

A fresh normal Replica connection uses current member and secret proofs,
the common purpose/capability exchange, an allowed path, the original
connection's fresh ACK and bilateral Ready before existing LogSync, record
verification, durable save/apply and sender receipt. Fresh foreground
consumer/provider work uses the smaller of the original absolute deadline's
remaining time and the existing operation ceiling. A later independent sync
on a maintained connection keeps its existing operation ceiling.

Purpose diagnostics distinguish restricted admission, Replica and business
on their own original connection. Runtime evidence qualifies only its
explicit SDK/platform/resource scope; relay success does not establish
physical NAT, OS routing or peer-to-socket association.
