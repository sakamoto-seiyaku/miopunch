# MIOPUNCH-62 evidence

Read the [validation record](../../notes/2026-09-30-legacy-nat-selector-validation.md) for behavior, limitations and reproduction. The authority is the [legacy migration contract](../../notes/2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md).

Final cases are exactly those enumerated by [kernel-verification.json](verification/kernel-verification.json). Each directory retains both node logs, the exact adapted fixture, WAN capture, kernel mapping/filter counters, conntrack and runner exit status. The verifier rejects missing observations, infrastructure timeouts and unimplemented execution counted as success. Capture occurrence counts can include the same packet on both WAN interfaces.

`eim-a`, `final-*` and earlier `verified-*` directories preserve earlier successful slices or investigations. In particular, `final-ipchange-b` is the retained executor-unavailable early-close failure; it is not an accepted NAT miss. `verified-ipchange-b` fixed the protocol failure but its original capture filter omitted alias `.10`; `verified-ipchange-capture-b` retains complete WAN evidence. `verified-ipchange-a-receipt` and five earlier captures had undrained libpcap buffers; the checker rejected them, and immediate-mode replacements are enumerated in the final JSON. Earlier artifacts are excluded from the final denominator, and no NAT success-rate claim is made.

Frozen binary hashes for the initial, intermediate and receipt-corrected batches are in `verification/mio62-*-runtime-sha256.json`. The final verifier's cases use the receipt-corrected `/tmp/mio62-verified-runtime` binaries. `verification/source-sha256.json` identifies the reviewed source after final formatting; formatting/test additions did not change runtime semantics. No fixture topology label or seed is supplied to the product classifier.

Reproduce a final case in the disposable kernel lab:

```sh
rtk cargo build --locked --bins --features nat-lab-test-hooks
rtk proxy docker run --privileged --rm --network none \
  -v /home/js/Git/miopunch:/repo:ro \
  -v /home/js/Git/miopunch/target/debug:/runtime:ro \
  -v /tmp/miopunch62-reproduction:/evidence \
  miopunch-nat57-lab:bookworm \
  python3 /repo/scripts/legacy_nat_lab.py positive eim a \
  /runtime/udp_nat_probe /runtime/udp_observation_probe /evidence/eim-a
```

Run `rtk proxy python scripts/legacy_nat_verify.py <case directories...>` for capture/trace verification. The checker requires expected exit codes and exact admission rejection reasons, the full peer capture filter, zero drops, captured/received equality and a matching complete pcap record count. Seven [negative checks](verification/verifier-negative.json) reject missing S, unrelated startup errors, unexpected exits, header-only or truncated pcap, undrained capture buffers and a wrong filter. The full workspace log includes all seven real QAD/TLS/UDP seam tests and real MQTT tests using the dedicated temporary broker; see [cargo-test.log](verification/cargo-test.log).
