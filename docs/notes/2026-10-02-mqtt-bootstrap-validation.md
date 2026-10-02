# Formal MQTT bootstrap and ordinary data release

MIOPUNCH-42 wires permitted saved MQTT resources into the native configured
endpoint. Two admitted nodes first authenticate and prove their current network
scope over real iroh QUIC/TLS through MQTT, then coordinate and validate UDP
on that original connection. `Endpoint::connect` and accept return an ordinary
Connection only after its data gate commits. Normal LogSync and a controlled
business stream then use UDP. The implementation follows the current
[ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/99acd13e-9980-45b0-a3ae-c29ec6babd2c)
and parent specification's approved node/Endpoint/Connection/stream seams.

The first real check used independent native identities and SQLite stores,
manual approval and durable receipt of both admission materials, then reopened
the nodes behind controlled Linux EIM/ADF NATs. Neither peer has a configured
public listener. A permitted standalone TLS QAD observer sees the same primary
UDP socket used for traversal and data; native relay is disabled. Successful
logs retain each endpoint's stable connection ID from restricted bootstrap
through real replica saving and business. WAN/LAN captures and NAT rules are
in [the evidence index](../evidence/mio42-mqtt-bootstrap/README.md).

## Recorded integration choices

The fork's defaulted `before_data_release` hook awaits path qualification after
the existing member admission hook but before ordinary candidate authorization
and connection release. The native hook reuses its AdmissionGate,
NatConnector and ControlSession. Both sides perform the existing authenticated
coordination; cancellation closes the original Connection. A validated physical
winner remains owned by that Connection without fabricating business success
for NAT learning. Existing explicit business-confirmation callers keep their
feedback rule. BBRv3 remains the existing stack implementation.

Formal startup is `NativeNetwork::bind_configured_endpoint[_with]`; the `_with`
variant supplies advanced socket/TLS settings while durable identity and saved
resource consent remain node-owned. The older advanced `bind_endpoint_with`
keeps callers' explicit carrier factories. Protocol users register their member
ALPNs with the AdmissionGate and Endpoint as before.

Each configured MQTT origin has a public 16-byte service tag and random 16-byte
delivery token, encoded together in a 32-byte CustomAddr. The tag qualifies the
broker-local route so multiple approved services can coexist. Only successful
SUBACK advertises a route; subscription loss withdraws it. A failed approved
broker cannot block another approved broker or a direct resource. The adapter
uses bounded queues of 64 entries, QUIC datagrams up to 2,048 bytes and MQTT
packets up to 4,096 bytes. QUIC provides stream fragmentation, reassembly,
duplicate handling and retransmission; an outer envelope carries `MIO1`, the
sender's 16-byte delivery token and one complete QUIC datagram. Its topic is
`miopunch/test/<delivery-token>/packet`, retained for local wire compatibility.
Invitation and private negotiation parameters are inside QUIC, not this envelope.

Endpoint snapshots share node learning, spent probe allowances and runtime
maintenance permission. Saving a UDP resource disable affects the next bind
without requiring Node.open or resetting budgets. Existing candidate freshness
and physical network-generation checks govern traversal.

## Evidence and limits

The genuine initial behavioral RED was no formal MQTT route. Later real REDs
found stale subscription advertising, a single-broker restriction and stale
saved UDP consent; each has a retained GREEN. Compilation and fixture failures
are separately identified and do not count as behavioral RED.

Subscription interruption closed the original TLS attempt on both sides with
no data handle and bounded failure; 100 early repeated calls did not add an
attempt. Independent subscribers replayed 97 distinct QUIC packets without
extra data connections. Real wrong-target TLS and another holder of the same
invitation were rejected; the legitimate member subsequently succeeded.
Simultaneous calls yield independent bounded Connections, rather than merging
the two callers' sessions. Cancellation closes the first attempt and a later
contact has a new stable ID.

After formal tagged startup, successful synchronization and business, a kernel
WAN DROP removed all peer UDP paths. 65,536 queued business bytes failed on the
original Connection after about four seconds; independent MQTT publication
counts stayed 98 to 98 on both sides. The observed close reason is
`LocallyClosed`, and there is no transparent redial. With UDP blocked from the
start, only control is reachable: no business handle returns, the MQTT route
remains available, and repeated early calls remain bounded.

This establishes native Linux controlled EIM/ADF startup with its allowed QAD
observer. It does not promise MQTT-only traversal of arbitrary NAT, public
broker availability, other-platform runtime, or arbitrary upstream wire/API
interoperability. MQTT-first data qualification in this increment is UDP-only;
unavailable or disabled UDP fails that attempt even if a native relay is
configured. A same-Connection native-relay upgrade is not claimed by the
declared no-relay acceptance environment. Legacy standalone probe routes remain 16-byte CustomAddr
values; configured native routes are 32 bytes and must be preserved in full.
The prior unknown-discovery Candidate's 16-byte legacy format is unchanged;
automatic composition of that discovery format with tagged formal startup is
not claimed here. Broader unknown-member recovery and client delivery remain
their own integration work. Service ACL, product UI and KCP are outside scope.
