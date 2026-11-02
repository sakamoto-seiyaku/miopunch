# Node contact listener: controlled lifecycle supplement

The latest focused run is `mio78-lifecycle-member-current-final`: six tests
passed through four commands with exit 0 and no guard termination or forced
kill. Build source snapshots match and both frozen executable hashes were
verified after execution. It covers two generation tests, the TLS negative
check and three maintained-node/carrier regressions. This run used the current
member custom-address eligibility fix and the new endpoint configuration seam.
The member caller/cleanup observations were 489/183 ms; the healthy original-M
disconnect/restart caller/cleanup observations were 420/125 ms. Generation
socket observations remained immediate=false, settled=6 ms, late=true.

After that recorded run, foundation review required retaining the complete
canonical `MqttFacility` in the native transport so saved-policy enforcement
can distinguish mappings sharing a native URL. The transport now exposes this
descriptor internally; raw and legacy carriers report none. The saved-policy
regression and current checks are owned by the shared SDK entry-point change.
The six-test receipt remains evidence for its recorded prior source version,
not a claim that this later descriptor change was tested by that run.

The frozen lifecycle supplement uses two test executables compiled from its recorded
SDK with `rtc-probe`. The build and both test commands exited 0; four tests
passed. The before/after build source snapshots match. Executable hashes were
verified before archiving; large test binaries remain in the temporary output
directory. Commands, actual exits, source and executable hashes are in
`mio78-current-frozen-supplement/receipt.json`; raw logs are stored as byte-exact
gzip files. `archive-manifest.json` records original and retained hashes.

This is an owned host loopback supplement. It does not qualify a public broker,
physical device pair, RTC data path, or public NAT path. No public traffic was
generated. Physical Pixel6a browser → Y700 maintained-native validation is
pending the shared SDK entry point and serial device window.

The maintained-node test performs real saved manual admission, then uses the
strict new member ALPN and common coordinator to establish an identity-checked
UDP session. An independently configured, real owned iroh relay remains
online. With a business activity lease active, cutting the MQTT connection
withdraws its route; the existing Node maintenance selector restarts exactly
one new generation. The original healthy business session continues PING/PONG
before the cut, during withdrawal, and after restart. Caller time was 399 ms;
Node and socket cleanup observation took 109 ms. UDP was immediately bindable
and remained bindable after the separate MQTT 100 ms late observation. MQTT
was empty immediately and at the late observation in this particular test.

The lifecycle test separately demonstrates current-generation readiness,
withdrawal before reuse, actual driver completion before restart, and rejection
of a deliberately late old-generation observer continuation against the new
ready generation. Its actual MQTT socket observation was **not immediately
empty**: it settled after 6 ms and remained empty at the 100 ms late check.
Shutdown joined the driver and retired observers, with zero queued packets;
terminal restart was rejected. The disabled-resource test opened zero MQTT
connections and refused a restart.

`mio78-generation-red` retains the original failing regression: dropping the
route allowed rebinding while its previous spawned driver was still owned.
`mio78-generation-green` retains the same regression after the completion
boundary fix. `mio78-node-supplement-first` retains an initial compile failure
caused by the shared facility type lacking `Eq`/`PartialEq`; it was not a
protocol failure. Earlier successful supplements remain as historical evidence
and do not add distinct coverage to those four tests.

The subsequent three-member story uses three actually admitted, independently
identified, reopened stores. The caller has no relay; the maintained receiver's
owned relay is online; a third member forwards restricted contact without data
relay permission. Its first sandboxed command exited 101 because socket binding
was denied, before the contact protocol ran. The same frozen executable then
ran with loopback permission and exited 101 with `DataPathUnavailable`. Narrow
boundary instrumentation reproduced the failure with two healthy forwarding
links and a 0 ms common connect rejection. Both raw failures, the diagnostic
run, and exact test source matching the compiled source hashes are retained in
the `mio78-member-*` directories. These failed runs do not claim graceful SDK
cleanup; their test processes ended.

The diagnostic found that the shared SDK checked `endpoint.addr()` for a
member custom address. The vendored endpoint's address watcher publishes only
IP and relay addresses; custom candidates use the separate `custom_addrs()`
getter. The shared module owner corrected that check while retaining the
Signaling permission guard. The fresh `mio78-member-tls-current` build then
passed a local TLS negative check and reached real native data success in the
member story, but its test cleanup failed because test-owned closed connection
handles were still retained. That exit 101 is preserved separately.

The subsequent `mio78-member-cleanup-current` build and single affected member
regression exited 0. Retiring the test-owned handles allowed all four forwarding
tasks to finish without abort; active forwarding links reached zero. Both
participants reported strict version 1, original-M path 1, fresh ACK and a
selected UDP path. Identity matching, reopened stores and PING/PONG were
checked. The maintained receiver's relay stayed online; the caller had no relay
or public MQTT configuration. Caller time was 493 ms and independent cleanup
observation was 181 ms. Three immediate socket observations were true; the
repeated bounded rebind sweep took 31 ms, and separate 100 ms late observations
were true. The sweep duration includes observation work. This member-only
result does not prove ordering when both member and public MQTT contacts are
present. The physical maintained-node public MQTT story is still required;
this ticket is not complete.

The Android public MQTT trust fix uses compiled Mozilla roots with the existing
ring provider and safe default TLS versions when `mqtts` has no explicit CA.
Other platforms and explicit CA input retain their prior behavior. An actual
in-memory rustls handshake using the same Android production configuration
rejected an untrusted self-signed server certificate with `UnknownIssuer`; SNI
remained enabled. That focused test exited 0 and generated no socket or public
traffic. Android compilation and actual public startup are separate evidence
owned by the shared fixture owner; this local check does not claim public TLS
qualification.

There is one Node maintenance selector. The contact adapter observes each
generation and provides explicit control; it does not run a separate reconnect
loop. Public IRC remains unavailable under its previously recorded slow
profile. Resource configuration saved by the existing API applies at the next
startup; this change does not introduce hot reload or choose public defaults.
