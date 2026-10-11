# Pixel Wi-Fi UDP destination diagnosis

The current MQTT/RTC failure has an independently reproduced network-path
failure: on Pixel's current Wi-Fi path, changing the destination on the same
UDP socket still produces a response from the first destination. The configured
Wi-Fi gateway runs Mihomo v1.19.19 with TUN `mixed`, `auto-route` and
`auto-redirect` enabled. This is sufficient to break the ICE checks observed
in the failed SDK captures. It does not isolate the particular gateway adapter
or upstream proxy component, nor establish that correcting the network will
resolve every possible SDK issue.

The earlier 2026-10-08 public-MQTT OP13 Native cellular → Pixel Browser Wi-Fi
story actually passed ordinary RTC business and nonempty Replica. Its Pixel
Wi-Fi address/gateway differ from the current test. The explicit STUN resource
is unchanged; the cellular interface address also changed. Phone-level
`NOT_VPN` and zero VPN agents do not exclude a proxy on the Wi-Fi gateway.
This corrects the interpretation of the
[current finite test](../op13-cellular-current-2026-10-11/README.md), without
changing or relabeling its original failures or qualification bounds.

The retained captures show address-observation replies on both phones, but no
DTLS records in either failed SDK attempt. In the Replica attempt, 240 Browser
ICE-check transaction IDs match replies from the configured STUN service;
the ordinary-member attempt has 239 such matches. Checks actually targeted
the Native private and observed mapped candidates, not that STUN service.
The replies lack MESSAGE-INTEGRITY. They are not valid ICE peer responses:
[RFC 8445 §7.3](https://datatracker.ietf.org/doc/html/rfc8445#section-7.3)
requires short-term credential authentication for requests and responses.
The old passing captures contain peer exchanges with MESSAGE-INTEGRITY attributes and DTLS.

An independent C program uses `socket`/`sendto`/`recvfrom` and random STUN
transaction IDs, without MQTT, SDP, WebRTC or the SDK. It first contacts A,
contacts B on that socket, then contacts B on a fresh socket. A/B are the
configured Cloudflare resource and the documented
[Twilio UDP STUN service](https://www.twilio.com/docs/video/ip-addresses).
Repeating with A/B reversed tests whether the first destination, rather than
one specific server, determines the erroneous reply source.

| Observation | OP13 cellular | Pixel current Wi-Fi |
|---|---|---|
| First destination, both orders | Requested endpoint replies | Requested endpoint replies |
| Second destination, same socket, both orders | Requested endpoint replies | First endpoint replies |
| Second destination, fresh socket, both orders | Requested endpoint replies | Requested endpoint replies |

All 12 probe transactions received matching transaction IDs. OP13 matches
all six requested destinations. Pixel mismatches both second destinations
on reused sockets, while both fresh-socket controls pass. Thus SDK regression,
SDP credentials or MQTT forwarding cannot explain this independent failure.
The packet evidence and probe together support Wi-Fi forwarding as a cause
of the current ICE failure; generic cellular NAT impossibility is unsupported.

No phone, gateway, proxy or SDK configuration was changed. Both phone root
IPv4 NAT/mangle snapshots have no REDIRECT/DNAT/TPROXY rules; this is a
current observation, not a claim about every possible kernel mechanism.
Non-root firewall reads failed; root read-only snapshots succeeded. Gateway
sudo reads were unavailable, while existing SSH/Docker access identified
the service and selectively read TUN settings. Credentials and unrelated
traffic were not published.

The C probe was built with the existing Android compiler/NDK, without Cargo,
and each deployed binary hash was checked before execution. All owned phone
probe directories were removed. Raw captures, addresses, private route
snapshots and source-address probe logs remain outside the repository;
`raw-references.json` binds originals to the published scalar projections.
The original parsing and probe inputs are included for reproducibility.

There is no product fix or new successful SDK retest in this dossier. A
repeat using the same pinned SDK artifacts on a Wi-Fi path that preserves
UDP destinations is needed before claiming restored mobile RTC business.
Network-path selection for such a repeat remains separate from the diagnosis.
