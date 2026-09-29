# MIOPUNCH-29 staged admission on the shared iroh endpoint

The shared `AdmissionGate` is installed through `Endpoint::builder(...).admission_gate(...)`; `NativeNetwork::bind_endpoint` installs it after loading the persisted device key and verified management view. The same handle can be passed to p2panda's `endpoint_builder_hook`, so Gossip's direct use of the underlying iroh endpoint receives the same check as p2panda `connect` and incoming protocol actors.

The gate classifies the **wire ALPN**, authenticated TLS peer, network and secret generation, completed SCRAM proof for this connection, and locally applied membership view. Unknown ALPNs fail closed. p2panda Discovery, Gossip, and LogSync wire ALPNs are the p2panda hash of their protocol ID plus the network ID; they are registered as member protocols. A node whose identity is known removed is rejected for every class until a valid authorization correction or rejoin changes the local view.

| ALPN class | No secret | Current secret, no membership | Current member | Candidate disclosure |
| --- | --- | --- | --- | --- |
| Admission request | Restricted entry only | Restricted entry only | Restricted entry only | Withheld |
| Genesis recovery | Restricted entry only | Restricted entry only | Restricted entry only | Withheld |
| Bootstrap/control | Rejected | Restricted entry | Restricted entry | Withheld |
| Discovery, Gossip, LogSync, test business | Rejected | Rejected | Normal protocol | Released after mutual stage agreement |

Admission request and genesis recovery ALPNs never authorize general history or business. Their handlers remain responsible for the invitation and bound root-credential checks before releasing material; the existing genesis-recovery handler performs its root proof. This ticket provides the connection qualification boundary, not a completed invitation workflow. A designated contact is checked against its real TLS identity by iroh when dialing; changing the target identity while retaining the same address fails TLS. A shared secret by itself grants only bootstrap scope.

The iroh fork registers **every** gated connection as restricted before running asynchronous `after_handshake` hooks. It disables 0-RTT and 0.5-RTT handles for gated endpoints. While restricted, the remote actor neither publishes local QNT candidates nor starts automatic hole punching. Only an explicit successful member decision calls the actor to release candidates; a default hook `Accept` is insufficient. The `AcceptRestricted` outcome keeps admission request, recovery, and bootstrap connections in the restricted state. Member ALPNs arriving on an unregistered custom path are rejected. For member connections first admitted over an allowed path, the actor and QUIC packet filter continue to reject later custom paths unless their data-capable carrier ID was explicitly allowed. The current TCP carrier is allowed by default; MQTT and future signaling carriers remain restricted without extra registration. Existing explicit control-only path promotion remains a separate concern.

The first bidirectional stream on a proof-requiring connection carries SCRAM-SHA-256-PLUS. After the proof, the second stream exchanges a one-byte mutual stage acknowledgment. For an admission request or genesis-recovery exception, the first stream is that acknowledgment. A protocol handler receives subsequent streams only after both sides agree; an asymmetric stale member view therefore cannot return a normal outgoing handle. The gate lends the completed same-connection proof through `proof_for` to existing signed-history handlers, rather than allowing a second proof attempt. It stores weak connection handles, cleans them on closure, bounds live sessions, and closes affected sessions when the applied authorization view or secret generation changes. Policy rejections do not report a failed network address to p2panda's address book.

Reproduce the real localhost UDP/TLS matrix and the MQTT carrier isolation case with a dedicated anonymous Mosquitto test broker:

```bash
rtk cargo check --locked --workspace --tests
rtk proxy env MIOPUNCH_MQTT_BROKER=127.0.0.1:PORT cargo test --locked --test admission_gate -- --test-threads=1
rtk cargo test --locked --test admission_proof
rtk cargo fmt --all -- --check
```

The `admission_gate` suite exercises a no-secret admission-request stream followed by Discovery/Gossip/LogSync/business rejection; a proved but nonmember bootstrap with a retrievable same-connection proof; member access to all four normal ALPNs; actual p2panda endpoint actor dispatch; persisted `NativeNetwork` endpoint configuration; TLS contact mismatch; known removal; asymmetric views; 0-RTT rejection; concurrent per-connection proofs; cancellation, timeout, rotation and reconnect; QNT candidate timing; a default no-op hook failing closed; real MQTT control allowed while initial and late business paths are rejected; and real TCP custom member data allowed. On 2026-09-29 it passed **16/16** with the broker at a temporary localhost port. The independent SCRAM regression passed **10/10**.

The gate contract is ready for later invitation, history, and message-carrier protocol handlers. They must register their ALPN class and check the corresponding live `allows` scope before releasing protected data. The existing restricted genesis-recovery and member-history handlers retain their own signed-history and root-proof validation.
