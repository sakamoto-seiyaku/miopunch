# portmapper provenance and delta

Source: crates.io `portmapper` 0.19.3, downloaded through
`https://crates.io/api/v1/crates/portmapper/0.19.3/download`.
Published VCS commit: `0030e57fc70981895bb86212ff0f059cc41c8a4e`,
`portmapper/` in `n0-computer/net-tools`.
The declared MIT OR Apache-2.0 license texts are included.

MIOPUNCH-39 adds `Client::deactivate_and_wait`: the service waits for cancelled
creation to finish its acknowledged-allocation cleanup, or releases an already
completed pending mapping, then releases its active mapping before acknowledging.
NAT-PMP retains cleanup ownership as soon as allocation is acknowledged, so
failure/cancellation of the subsequent public-address query cannot lose it.
The caller bounds shutdown; no foreign gateway mappings are enumerated/deleted.
Gateway discovery, NAT-PMP wire encoding and successful lease renewal/expiry
remain the upstream implementation. Failed renewals conservatively withdraw
the candidate and release any remaining owned mapping. Native product configuration opts into
NAT-PMP only; PCP/UPnP runtime capability is not claimed by this ticket.

MIOPUNCH-41 retains each completed NAT-PMP Mapping's TCP/UDP protocol and
uses it for release, matching the pending-allocation cleanup guard. Upstream's
completed release was UDP-only even for a TCP allocation. No gateway table is
cleared: deletion remains keyed by the owned local IP, protocol and port.
