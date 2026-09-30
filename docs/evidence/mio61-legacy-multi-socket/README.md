# MIOPUNCH-61: physical multi-socket legacy NAT evidence

The frozen 38-cell matrix completed. Independent verification passes all 38:
19 original-connection business confirmations through a real nonprimary
binding, 15 complete bounded misses, and four admission rejections. These
are acceptance classifications, not a 38/38 NAT success rate.

The primary matrix contains 30 trials: five mapping topologies, both original
QUIC dialer roles, and all three old mode indexes. It yielded 18/30 business
confirmations and 12/30 natural misses. The auxiliary cells add two blocked
misses, one successful and one missed unsupported-TTL trial, and four
admission rejections. No extra seed or random trial was added to improve the
results. `mio61-frozen-01` fixes the fixture's allocator schedule; the product
uses its ordinary thread RNG and these trials are not claimed bit-replayable.

| Topology | Business | Bounded miss | Raw phase ms, both peers | Probes per peer | Allocator collisions per peer | Pool-exhausted tuples per peer |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| eim-random | 6/6 | 0/6 | 3002–3132 | 4–605 | 0–8961 | 0–557 |
| eim-wide | 6/6 | 0/6 | 4091–15780 | 91–16565 | 0–0 | 0–0 |
| up1-random | 6/6 | 0/6 | 3003–9735 | 5–1285 | 0–19404 | 0–1205 |
| up1-wide | 0/6 | 6/6 | 38000–38002 | 1285–1700 | 0–0 | 0–0 |
| up1-up1 | 0/6 | 6/6 | 38000–38001 | 1285–1700 | 0–0 | 0–0 |

Each ratio contains only the six normal positive trials of that topology.
`verification/costs.json` retains each role's recorded costs and available MQTT
counts. Per-process CPU was not measured by the original collector and cannot
be reconstructed after those processes exited; that required cost metric is
explicitly absent. The 16-port fixture models explicit collision/exhaustion drops rather
than silently rewriting a requested SNAT port. Those losses are part of its
controlled distribution and are not a claim about a real router's allocator.
The full-port fixture allocates 1024 through 65534. All source sockets enter
the allocator, and full `(source IP, source port, destination IP, destination
port)` keys are passed to real kernel SNAT and reverse conntrack filtering.

`canonical/<cell>/` contains the exact fixture, product logs, allocator costs,
three full WAN/LAN captures, capture shutdown counters, conntrack, and the
independent preflight of two physical source ports and wrong/right return
tuples. The checker matches every actual raw datagram to its physical local
binding and allocator outcome; it also checks round, nonce, TTL, phase
ordering, random draw bounds, reply bursts, old delay and absolute budget,
original peer/stable ID, selected QUIC path, and protected business feedback.
The corrected handoff generations additionally require both original peers'
winner-ready messages within the remaining original deadline.

Large streams are stored as deterministic gzip without truncation or sampling.
`verification/compression.json` records their original and compressed hashes
and sizes. The reader verifies both hashes and the complete stream inventory;
the 38-cell strict result is byte-identical before and after compression.
`costs.json` also counts actual outgoing QAD datagrams and non-traversal peer
QUIC datagrams and UDP payload bytes from each full LAN capture, excluding the
separately tagged preflight traffic. These are product-lifetime capture costs,
including observation and path validation rather than only business payload.

The 3000 ms check derives the shared phase-start reference from the receiver's
first captured request minus its reported socket-bind elapsed time. Socket
setup consumes the original 38000 ms budget. Comparing only the peers' first
raw packets would erase approximately 2.6 s of 256-socket setup and wrongly
fail a sender that waited the full 3 s. A 250 ms observation tolerance covers
registration/packet scheduling; the absolute phase-stop and ready-deadline
checks remain separate. The prior checker and before/after hashes are kept
in `verification/delay-checker-*`.

The production changes were corrected during the fixed sequence. Canonical
cells 1–12 use runtime generation 1, 13–18 generation 4, and 19–38 generation
5. `verification/runtime-by-cell.json` maps every actual fixture hash to its
runtime and source manifest; the binary and dummy pair hashes are retained.
The final current-source suite verifies the later handoff and epoch fixes.
The manifests describe actual execution versions; earlier cells are not
misrepresented as executions of the final binary.

Earlier failures remain under `runtime-failures/`: the old 3 s automatic
handoff window, a retrospective post-business guard, and periodic QAD reports
incorrectly counted as network changes. The infrastructure log-creation race
is retained separately. Each affected fixed cell was retried only after the
identified correction. Generation-5 cell 19 itself was not rerun: the original
protected PING/PONG and stopped acknowledgement printed the configured
`EARLY_FAILURE_CONTROL_ALIVE` marker, while the wrapper expected another
equivalent marker. Its original harness exit and explicit validator repair
are preserved in `verification/cell19-validator-repair.json`.

The final real-broker serial suite has 236 passing tests, zero failures and
three existing ignored tests (`verification/cargo-test-epoch-final.log`).
The separately ignored CPU entry was added after that suite and executed only
in the frozen functional supplement. The suite includes real partial bind failure, 256-descriptor cleanup across rounds,
per-packet TTL/source observations, cross-socket authenticated receipts,
ordinary send errors, foreign original connections, path disposal, protected
winner handoff, and physical epoch invalidation. Sixteen deliberate evidence
corruptions are rejected (`verification/verifier-faults.json`): truncated or
dropped captures, early delay, wrong wire TTL, socket merging, wrong winner,
deadline reset, early miss, lost control, wrong random count, admission work,
wrong rejection identity, changed exits, and allocator bypass.

The same existing MQTT control connection remains available after each miss.
The diagnostic traffic snapshots are actual adapter counts before the final
liveness exchange; success logs record the packet count at business start.
Success traffic has no complete byte snapshot, so no such byte total is
inferred. No heartbeat or probe budget was increased to obtain these results.
Normal data remains barred from the MQTT-only carrier.

Runtime packet evidence is Linux/Docker only. Native and browser WASM compile
checks passed; browser UDP traversal, Android packet behavior, and real-router
success rates are not claimed. MIOPUNCH-60 remains responsible for automatic
selection and node-maintenance integration; this ticket does not enable new
automatic strategies by itself.

The missing CPU recording is supplemented by exactly three separately frozen
functional measurements, rather than replaying or changing the historical
NAT trials. `verification/cpu-plan.json` and its SHA-256 freeze the test binary,
43 source inputs, image, mode/index, counts and budgets before execution.
Each fresh Rust process contains two real admitted nodes and two QAD servers.
Explicitly synthetic loopback observations select the normal old mode 0, 2
or 4 at index 0. A disposable loopback-only container drops just raw traversal
datagrams on INPUT after their successful kernel submission; preflight and
actual counters prove no-hit enforcement. QAD, original QUIC and protected
control remain live. The prior OUTPUT preflight returned EPERM; its correction
was completed before any measurement child ran and is recorded separately.

| Functional case | User CPU s | System CPU s | Process wall s | Actual raw drops | Actual phase stops ms | FD before/after |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| mode 0/index 0 single-socket baseline | 0.527418 | 0.058140 | 5.880 | 51 | 5200 / 5201 | 16 / 16 |
| mode 2/index 0 complete random/listener work | 13.649365 | 5.111731 | 38.691 | 46839 | 38001 / 38001 | 16 / 16 |
| mode 4/index 0 complete random/listener/range work | 2.816101 | 0.629761 | 38.684 | 2635 | 38000 / 38010 | 16 / 16 |

These are actual `wait4` user/system measurements of the whole isolated
two-node debug-test process, including setup, QAD/TLS, two Tokio workers,
control maintenance, executor and diagnostic output. They are not per-peer
CPU or estimates for a production build or device. Both full cases completed
1000 random iterations, 256 bind attempts/successes and their original 38 s
phase; mode 4 emitted its inclusive five-port range. Every case's raw INPUT
counter equals its submitted probes, descriptor count returns to 16 and the
same two original stable IDs exchange protected PING/PONG afterwards.
`verification/cpu-supplement/` retains raw logs, counters and process rusage.
No CPU process was repeated. These three measurements add zero NAT probability
samples; CPU for the original 38 cells remains `not_measured`.

Run the offline checks from the repository root:

```sh
python3 -B scripts/legacy_nat_multi_socket_verify.py docs/evidence/mio61-legacy-multi-socket
python3 -B scripts/legacy_nat_multi_socket_verify_faults.py docs/evidence/mio61-legacy-multi-socket
```
