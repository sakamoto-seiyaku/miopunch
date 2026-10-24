# Native TCP/UDP forwarding and bounded browser access

MIOPUNCH-118 implements the non-UI port-forwarding operations of MIOPUNCH-107.
These are engineering choices within its existing node → service authorization
model. Platform qualification remains separate from source delivery.

The provider explicitly enables TCP and UDP separately, supplying up to 32
literal socket addresses for each protocol. Only these exact protocol/address
pairs are accessible. There is no implicit arbitrary-host proxy, DNS resolution,
wildcard target or second ACL. Its existing signed service declaration and
single effective service rule apply. Target scopes are current runtime settings;
reopen starts disabled and requires an explicit target list again. Enabling or
replacing a scope preserves the saved ACL and cancels flows whose target ceased
to be allowed. These choices avoid a new resource hierarchy or permission engine.

NativeClient exposes `enable_forwarding`, `disable_forwarding`,
`start_forwarding`, `stop_forwarding`, `forwarding_status`,
`forward_exchange` and `cancel_forwarding_flow`. A consumer's local listener
binds an explicitly selected loopback address, including an ephemeral port.
It does not require enabling its own provider. The returned `listen` confirms
that local bind; each application's connection still needs the remote provider
and target to succeed. Status distinguishes listener state, active flows,
transport observations and bounded failure counters. The JSON-lines host offers
`forward_enable`, `forward_disable`, `forward_start`, `forward_stop`,
`forward_cancel` and `forward_status` for the same module.

Each TCP application connection gets its own admitted SDK connection and framed
duplex byte stream. Clean FIN propagates a half-close; a truncated record or
reset is still an error. A provider that has finished both TCP directions keeps
its transport owner until the consumer has read the final response/FIN and
closed the connection, with the same idle bound and immediate policy cancellation.
This preserves a target response produced only after request half-close.
UDP uses a separate admitted service protocol. Each
local UDP source gets one connected provider socket and a bounded queue; empty
datagrams retain their meaning, and the native wire supports datagrams up to
65,507 bytes. Multiple replies and unsolicited target datagrams can return
through that active association. There is no request/response assumption in
the native listener. Queue overflow or a host datagram above the wire limit drops only that
datagram and increments its counter; it does not retire the listener. Native
host receive buffers cover a full ordinary UDP payload (65,536 bytes), then
apply the smaller wire bound. A consumed Windows WSAEMSGSIZE is also one packet
drop; unrelated socket errors remain errors. The same receive classification
applies to provider replies, with drops visible on the active flow instead of
ending that continuing association.

The runtime allows at most eight listeners and sixteen active flows per node,
with at most sixteen queued UDP datagrams per association. Provider receivers
have a separate sixteen-worker bound and transfer out of the generic ingress
handler after admission, so full forwarding capacity leaves signed service
policy/control synchronization usable. Disable and target replacement await
the affected receiver owners after cancellation. Each stream record
is at most 64 KiB. Connection/control setup has a finite budget; socket and
stream I/O have a 60-second idle bound. Stop cancels pending connection work,
closes accepted sockets, and joins the listener's children before returning.
Dropping an operation owns its connection cleanup. Node close cancels all
module owners and joins them with the existing dispatcher. There is no new
general scheduler or transport runtime. The existing Native host listener also
awaits endpoint shutdown after cancellation and handler joins, keeping QUIC
close notifications alive long enough for peer sockets to finish when the host
is dropped; explicit host close uses that same teardown.

Actual provider execution checks its current membership, effective service ACL,
explicit enable and exact target before opening the target socket. Existing
service publication immediately cancels flows affected by a rule/scope change;
the existing maintenance loop refreshes guest deadlines and local authority.
Service-scoped guest invitations use the existing signed invitation, first
identity binding, exact-live grant projection and per-service admission gates.
A TCP guest does not obtain UDP, Files or generic member traffic. Guest exit,
revocation and expiry end affected forwarding without extending the grant.
`join_guest_with_resources` also preserves a caller-supplied resource allow-list
through initial startup, persistence and reopen; the ordinary guest operation
retains its existing default policy. The CLI accepts that optional resource
configuration for the same operation.

BrowserNetwork exposes `forwarding_status`, `forward_exchange` and
`stop_forwarding`. It offers a bounded TCP request/response or one UDP datagram
through a capable Native provider. Input and output are each limited to 60 KiB,
with four concurrent exchanges. The TCP caller supplies the expected response
length; arbitrary persistent protocols use the native listener. Browser status
explicitly reports no native host listener and no provider capability. Browser
roles, identity and ordinary service authorization are unchanged.

Validation consumes immutable Linux/WASM/Windows pins from the shared build
cache; technical hosts use only owned sockets, generated bytes and private node
directories. The local environment's `127.0.0.1` large UDP observations failed
even without the SDK; alternate owned IPv4/IPv6 loopback observations preserve
that boundary. Qualified IPv6 tests retain the same exact datagram assertions,
including empty and maximum-size packets. This is local functional evidence,
not WAN/NAT, every IPv4 route or actual Windows qualification. Real Windows
execution and provider-role exchange remain M118 requirements pending an
applicable grant; cross-compilation does not satisfy them.

The later defensive receive and reopened-test-reporting changes were made under
an explicitly offline scope. Their compilation, pure value checks and static
review are separate from functional qualification. No new UDP boundary/reply or
SDK networking scenario was run for those changes; those runtime obligations
and actual Windows qualification remain open.
