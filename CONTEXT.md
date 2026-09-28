# miopunch Context

Status: design in progress. This is a reading map, not a finalized product
specification. The [product direction draft](docs/notes/2026-09-27-product-mainline-draft.md)
separates confirmed preferences from proposals; the
[scenario walkthrough](docs/notes/2026-09-28-network-scenario-walkthrough.md)
records the concrete behavior discussed so far.

## Working language

- **Node identity** is stable across address, network, and transport changes.
  It is distinct from the address or path currently used to contact a node.
- **Membership and authorization** describe who belongs to a network and what
  they may do. A discovery or relay facility does not grant either by itself.
- **Contact/discovery** finds candidate peers and conveys protected connection
  messages. It does not necessarily carry application data.
- **Data path** carries the end-to-end session, directly or through an allowed
  relay. Signaling forwarding and application-data relaying are separate uses.
- **Local network state** includes verifiable membership, authorization, and
  contact information; a contact list is only one view of it.

## Confirmed direction

The network should remain useful without requiring every user to run a VPS or
every pair of members to maintain a permanent connection. Members may retain
and forward verifiable state so an offline node can catch up when it returns.
Connection maintenance should try to preserve the same end-to-end session
across allowed path changes, but a session that cannot be maintained must fail
clearly. Official facilities must not become an inescapable dependency.

## Open design

The invitation and membership-proof flow, pre-connection signaling protection,
state synchronization choice, resource policy, service ACL, and concrete iroh
and p2panda integration are not settled. Reuse or replacement of the archived
tests and lab scripts is also undecided. Research in `docs/notes/` informs
these decisions but does not decide them. Record accepted, hard-to-reverse
decisions in `docs/adr/`; track unresolved decisions in Plane.

The prior Go/Android implementation and OpenSpec material are archived at
`archive/legacy-go-openspec-2026-09-28/` and do not define the new baseline.
