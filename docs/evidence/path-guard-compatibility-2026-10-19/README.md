# Ordinary paths and explicit final guards

[MIOPUNCH-103](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/102f6353-93ef-4093-a614-057b3a1326fa)
found and fixed an actual buffered-send permission leak. A sender held 47 bytes
of encrypted STREAM while Pending; changing only the installed application
predicate to false still delivered `buffered-forbidden` to the peer. The valid
pre-fix assertion ran after normal endpoint and UDP-port cleanup.

The protocol now retains private actual-frame permission metadata on each
SocketTransmit, combining coalesced packets/GSO datagrams and resetting each new
transmit. The async sender checks the current application predicate before every
attempt. Mixed batches require permission; validation/control exceptions remain.
Original Transmit fields, ordinary constructors, required sender methods, wire
encoding and upstream default features remain unchanged.

Implementation: `7780db8970fb10cdd2158d6d07f9749c8102f482`, based on
`70e5d5522deb558fca88b9ba0087dc0376fd8091`.

Four new guard/consumer tests, 17 protocol tests, 12 actual socket tests and 15
unified connection tests passed: **48 total**. The unchanged original Iroh
consumer passed IP, owned HTTP relay and legacy custom UDP streams on published
Iroh 1.2.0 before edits and on the final fork. The existing unchanged Noq consumer
passed unrestricted single/healthy multipath on published Noq and the final fork.
Final application-only revocation: forbidden ciphertext carrier repolls **0**,
plaintext delivery **none**; a separate held PING was actually received and ACKed.

Rebuilt Native/Browser SDKs completed the current whole flow over owned loopback
TLS/WSS and relay: restricted request (~659 ms in this run), one approval, durable
authorization and secret, both same identities reopened, member `PONG`. Native,
Browser, origin, relay, UDP sockets and containers closed normally. Generated
private test keys/NSS were removed only after actor and container cleanup.

[Validation](validation.json), [selected input continuity](selected-input-continuity.json),
[work units](work-unit-measurements.json), [independent reviews](review.md), and the
[raw archive manifest](archive-manifest.json) retain the exact evidence. Every
archive member was read back and hash-verified. The archive includes original
sources/locks, frozen selected source copies and depfiles, preconditions, Cargo
logs, valid failure/success receipts and public fixture controllers. SDK runtime
logs, invitations/material, private receipts, keys, NSS and stores are excluded;
the public files and every archive member are scanned against actual private
invitation/material values. SDK binaries/WASM remain raw local pins with hashes.
Obsolete test programs have lossless gzip roundtrip/restore receipts; no SDK raw
pin, dependency library, profile, lock dependency row or toolchain was changed by
cleanup. Existing 249 WIP files retained their exact hashes and dirty status.

Preparation failures are retained and distinguished from product RED: an early
ordinary consumer read an upstream private ECN field before its first qualified
baseline; the first Noq fixture assumed immediate async socket release; a new
probe fixture initially called a nonexistent Connection::ping rather than the
existing Path::ping; a Docker attempt lacked socket access. A historical Noq
selector had been expanded before M103: its exact original manifest was recovered
and verified, and all actual selected sources/raw artifacts remained unchanged.
The failed freeze started no actors and did not relabel that historical graph as
a current graph. Earlier guard/SDK passes without the application-only revocation
case remain historical, separate from final rebuilt acceptance.

Counts measure real callback/method/poll/datagram/transmit/byte entries, including
actor and packet work and transparent counter cost. Zero RTC-policy calls on IP
does not imply zero internal gate cost. No CPU time, per-packet cost or performance
SLA was measured. Public, physical-device and all-platform matrices remain under
their existing separate acceptance scope. Standards: 0 findings; Spec: 0 findings.
