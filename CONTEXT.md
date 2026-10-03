# miopunch Context

Status: the behavioral baseline is specified; concrete integration and validation
remain open. This glossary and reading map accompanies the current
[implementation specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
The [product direction draft](docs/notes/2026-09-27-product-mainline-draft.md)
separates confirmed preferences from proposals; the
[scenario walkthrough](docs/notes/2026-09-28-network-scenario-walkthrough.md)
records the concrete behavior discussed so far.

## Working language

- **Network identity** identifies a private network tied to its genesis
  credential. A new genesis credential means a new network, not an in-place
  credential rotation that preserves the old network's identity.
- **Genesis credential** is the verifiable basis of a network's unique genesis
  authority, independent of any fixed device. Restoring the same credential
  from backup does not create a new network; it can restore management authority
  on a new node without approval from an old administrator, even when all old
  administrator devices are unavailable.
- **Network admission secret** is a shared network secret used to gate bootstrap
  access, distinct from node identity and the genesis credential; possession
  alone does not establish current membership or grant business or management
  permissions. Former holders may retain bootstrap access under a new node
  identity until rotation; known revoked identities remain rejected.
- **Admission-secret delivery** gives an approved joining node the current
  network admission secret over its secure connection after manual approval or
  successful precode redemption; invitation material does not itself contain
  that long-lived secret. Daily administrators may deliver the current secret
  to approved members without gaining authority to rotate it.
- **Admission-secret rotation** replaces the network admission secret without
  changing network or node identity and requires genesis elevation. It is
  separate from member removal, with an optional combined removal-and-rotation
  operation; switching requires retained members to confirm durable receipt of
  the new secret, otherwise waiting or explicit removal of unconfirmed members.
  New admission processing and admission-secret delivery pause during rotation;
  distribution of the replacement secret to retained members is part of rotation.
- **Daily administrator** is a node authorized to approve ordinary members,
  issue ordinary-member invitations, and remove ordinary members without
  using the genesis credential. Granting or revoking administrator status,
  or removing a node that still has that status, requires genesis authority.
- **Administrator revocation** removes administrator status while retaining
  ordinary membership; removing the node from the network is a separate act.
- **Genesis elevation** is temporary genesis authority verified using the
  genesis credential, scoped to the current device and management session.
  Like sudo, it permits repeated privileged operations within a temporary
  window; expiry or explicit exit requires verification again.
- **Management recovery** restores administrator authority on a new node
  using the existing genesis credential without automatically revoking old
  administrators; their revocation is an explicit, separate operation.
- **Network migration** is a transition to a new network, with each device
  confirming its own participation once and validating the destination before
  switching and leaving the source. A smoother transition does not make the
  destination the same network as the source.
- **Single active network** limits normal node operation to one network in the
  current scope. Migration may temporarily retain both networks' authorizations
  while preparing the destination, without offering ongoing dual-network use.
- **Migration preparation** obtains and saves destination admission material and
  verifies usable communication with a legitimate destination member while the
  source remains active. Failure before switching leaves the source in use;
  completed departure is not silently undone.
- **Node identity** is stable across address, network, and transport changes.
  It is distinct from the address or path currently used to contact a node,
  and is retained when that node migrates to a new private network.
- **Invitation trust** starts with invitation material personally passed through
  a trusted channel, identifying the intended network and designated contact;
  replacement of that entire invitation is outside the current threat scope.
- **Private signaling** is protected for the intended pair of independently
  identified nodes. Sharing an invitation does not merge node identities or
  let another holder decrypt or impersonate either endpoint in their exchange.
- **Control bootstrap** establishes a restricted, protected exchange between
  identified endpoints before a path is ready for normal synchronization and
  business traffic; node authentication does not itself grant network admission.
- **Membership and authorization** describe who belongs to a network and what
  they may do. A discovery or relay facility does not grant either by itself.
  A migrating node obtains membership in the destination network anew, with
  authorization evaluated independently in that network.
- **Membership lifetime** has no automatic expiry in the current scope;
  temporary membership is out of scope.
- **Local authorization state** is the membership and authority view derived
  from the valid management records currently available to a node. It may
  change after missing records arrive and conflicts are resolved, including
  invalidation of a previously applied member removal; local effect is not
  irreversible finality.
- **Admission success** means the joining node has received, verified, and
  saved both its membership authorization and the network admission secret,
  whether admitted by manual approval or precode redemption; it does not
  require network-wide synchronization or reachability of every member.
- **Pending admission completion** means approval has succeeded but the joining
  node has not yet saved all required admission material. Interrupted delivery
  may resume from the original delivery node for the same authenticated node
  while its authorization remains valid, without new approval or another
  precode usage being consumed.
- **Admission completion via a new precode** uses a fresh invitation from
  another administrator when the original delivery node is unavailable,
  consuming a use of the new code without taking over the original redemption.
  Existing valid membership is retained and its admission material delivered;
  otherwise ordinary admission rules apply, without blindly adding the same
  member again.
- **Disconnection** temporarily stops participation while retaining membership,
  allowing the node to reconnect without new admission authorization.
- **Leaving a network** voluntarily ends the node's own membership and requires
  new authorization to rejoin; a daily administrator must use genesis elevation
  to leave. Other members may retain and forward the verifiable departure
  decision.
- **Rejoining** establishes new membership authorization for a previously
  removed node while retaining its node identity. It requires fresh approval
  or redemption of a new valid invitation; old authorization or a retry of the
  old admission request cannot restore membership.
  Correction of a removal later found invalid is an authorization-state
  correction, not rejoining after a valid removal.
- **Member removal** revokes membership: a node that receives and verifies
  the decision rejects new member connections and terminates existing network
  and application communication with that member. Nodes that have not received
  the decision remain subject to the accepted propagation window.
- **Precode** is a preauthorized invitation bound to one designated redemption
  node, including invitations limited only by time; it does not bind an applicant
  identity at issuance. Its expiry and optional usage limit constrain new
  redemption, independently of membership already granted through it.
- **Designated redemption node** is the sole node authorized to redeem a given
  precode and manage its individual revocation; other nodes may forward requests
  and retain or relay granted authorizations. Redemption requires administrator
  status: losing that status invalidates its old codes, and regaining it does
  not revive them; unavailability prevents new redemption without automatic
  takeover, while membership already granted remains independent.
- **Contact/discovery** finds candidate peers and conveys protected connection
  messages through supported channels such as MQTT, IRC, or member forwarding.
  The channel does not confer trust or decryption rights and does not necessarily
  carry application data.
- **Default resource configuration** supplies usable contact and address-observation
  resources with the client and is maintained as those services change. Users may
  replace or disable resources; configuration updates do not override those choices.
- **Node maintenance policy** sets a node's own background contact and path-exploration
  effort. Mobile devices may favor lower consumption while always-on routers or
  gateways may work more actively, without changing their identity or permissions.
- **Browser client** is an independently identified network participant with the
  same admission and membership semantics as native clients. Its initial connection
  capability requires a browser-reachable public or self-hosted relay; direct paths
  can be added later.
- **Data path** carries the end-to-end session, directly or through an allowed
  relay. Signaling forwarding and application-data relaying are separate uses.
- **Local network state** includes verifiable membership, authorization, and
  contact information; a contact list is only one view of it.
- **Control-state history** retains the shareable network-control and authorization
  records a node is entitled to obtain, including the dependencies needed for
  verification. History compaction is deferred until practical storage or performance
  pressure warrants a safe solution.
- **Synchronization completion** describes verified and saved records for the
  sources and scope of a particular exchange, not proof of globally latest state.

## Confirmed direction

The network should remain useful without requiring every user to run a VPS or
every pair of members to maintain a permanent connection. Members may retain
and forward verifiable state so an offline node can catch up when it returns.
Connection maintenance should try to preserve the same end-to-end session
across allowed path changes, but a session that cannot be maintained must fail
clearly. Official facilities must not become an inescapable dependency.

## Open design

Implementation planning now defers product UI and omits KCP. TCP carrying QUIC
is an accepted direction; use the selected stack's existing congestion-control
implementations (currently including BBRv3), without a Brutal port or custom
congestion extension in this scope. Validate in small runnable increments, with
real failure cases early. The available Docker/NAT lab materials and ADB-connected
Pixel6a support this approach; see the
[transport and validation follow-up](docs/notes/2026-09-29-transport-and-ui-followup.md).

The [shared Web and client UI discussion](docs/notes/2026-10-03-shared-web-client-ui-direction.md)
records the preference for code reuse and lower long-term maintenance cost,
with UI still deferred. Vue and Tauri remain a recommendation, not an accepted
framework decision or an instruction to begin implementation.

The detailed admission-proof mechanism, unknown-peer discovery messages,
signaling-carrier integration, and concrete iroh/p2panda composition still need
technical work. Resource-selection details, runtime policy changes, client
delivery, and persistence boundaries remain subject to the specification's
validation prerequisites; service ACL design is paused. Reuse or replacement
of the archived tests and lab scripts is also undecided. Research in
`docs/notes/` informs these choices but does not decide them. Record accepted,
hard-to-reverse decisions in `docs/adr/`; track unresolved work in Plane.

The prior Go/Android implementation and OpenSpec material are archived at
`archive/legacy-go-openspec-2026-09-28/` and do not define the new baseline.
