# noq packet-admission fork

Based on the published crates.io `noq` 1.3.0 package, registry SHA-256
`b78be567e796cfa74bb9bdc4117790af2d505eb887019cf8803244353eb09d89`.
The package's `.cargo_vcs_info.json` names upstream commit
`c1f411562e6078852749b8bcf1190523096a107f`, path `noq`.

MIOPUNCH-21 changes only `src/connection.rs` and its public re-export in
`src/lib.rs`: a monotonic per-connection path predicate runs under the
connection-driver lock before accepting an inbound datagram and before giving
an outbound packet to the transport. It clears a buffered packet that was
already waiting on a newly forbidden path and counts rejected packets.
This is a packet path gate; it does not inspect or decrypt application frames.
The dependency is selected by the root `[patch.crates-io]` entry.
