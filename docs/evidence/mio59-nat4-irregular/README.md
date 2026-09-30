# MIOPUNCH-59 evidence

Result and scope: [validation report](../../notes/2026-09-30-nat4-irregular-validation.md). This closes the revised real-lab/frozen single-candidate benchmark and negative acceptance. It does not implement old random1000/listen256 (61) or complete default routing (60). The full old selector62 and complete mode0/1/3 executor63 remain intact.

## Canonical frozen measurements

| Directory | Complete trials | Frozen outcome |
| --- | ---: | --- |
| `matrix/` | 256 | local16 irregular×stable auto4/64 vs baseline0/64; irregular×+1 auto1/64 vs baseline0/64 |
| `wide/` | 32 | whole-pool×stable and ×+1, each arm0/8 |
| `dual/` | 32 | local/local and whole/whole, each arm0/8; automatic outcomes are zero-peer-probe historical fallback |
| `tuning/` | 64 | separate padding0=0/32, padding4=1/32; no optimum claim |

Each preserves `trials.{json,csv}`, grouped `summary.json`, per-cell logs/conntrack/rules/WAN captures, independent `verification.json`, and `timing-and-costs.json`. All384 WAN captures fully drain with captured=received, complete untruncated records and zero kernel drops. Historical measurements have no physical LAN captures. Five exact post-PASS iproute2 namespace-reference teardown warnings are preserved and reported; no unexpected endpoint/allocator error is accepted.

Run the independent checks from the repository root:

```sh
rtk proxy python3 -B scripts/nat4_irregular_verify.py docs/evidence/mio59-nat4-irregular/matrix 32 random-eim,random-up1
rtk proxy python3 -B scripts/nat4_irregular_verify.py docs/evidence/mio59-nat4-irregular/wide 4 wide-eim,wide-up1
rtk proxy python3 -B scripts/nat4_irregular_verify.py docs/evidence/mio59-nat4-irregular/dual 4 random-random,wide-wide
rtk proxy python3 -B scripts/nat4_irregular_verify.py docs/evidence/mio59-nat4-irregular/tuning 8 random-eim,random-up1 padding0,padding4
```

## Current merged-version supplement

`verification/safety.json` contains19 exact safety acceptances; `verification/compatibility.json` contains17 representative acceptances, including one deterministic original-Connection QUIC business positive. These are supplemental checks, not new success-rate measurements. All required physical LAN-A/LAN-B and WAN pcaps are complete with zero drops. The fixture also checks wrong-source-IP filtering and the same source socket across different destination IPs. Protected liveness uses the original stream/stable ID; known revocation instead performs its precise safe connection termination.

```sh
rtk proxy python3 -B scripts/nat4_irregular_acceptance_verify.py safety docs/evidence/mio59-nat4-irregular/safety-final --retry-map docs/evidence/mio59-nat4-irregular/verification/safety-retry-map.json --output /tmp/mio59-safety-verified.json
rtk proxy python3 -B scripts/nat4_irregular_acceptance_verify.py compatibility docs/evidence/mio59-nat4-irregular/compatibility --retry-map docs/evidence/mio59-nat4-irregular/verification/compatibility-retry-map.json --output /tmp/mio59-compatibility-verified.json
```

Canonical evidence is `safety-final/` and `compatibility/`, except exactly two declared replacements under `acceptance-retries/`. Original budget evidence fired the freshness guard before the budget guard; original deterministic positive had6 WAN kernel drops. Retry maps preserve and validate both originals and exact case/seed replacements. Capture-only retry uses the same binary, with8192KiB buffer and4096 snapshot; caplen=original remains mandatory. No random trial was rerun to chase a hit.

`resume-smoke/` is the first complete blocked-WAN liveness/capture check. `safety/` is the initial noncanonical expansion, including wrong-identity/revocation expectations that were corrected and a network notification tested before actor invalidation. `exploratory/` retains earlier incomplete fixture/tuning discovery. These directories are historical diagnostics and do not contribute to the canonical384 measurements or36 supplemental cases.

## Provenance and checks

- `verification/runtime-sha256.json`: six independent snapshots,32 files, including imported `nat4_pair_matrix.py` in the three historical snapshots. Source/binary versions are kept separate. Original text manifests remain.
- `verification/preserved-59.patch.gz` and `preserved-sha256.json`: saved59 WIP at original base2be611dbf23710c8aa23685c7d648f745ebfd396; both complete source copies remain in `/tmp/mio59-preserved-before62-2026-09-30`. Historical compilation iterations can differ from the final saved source; exact runtime hashes identify measurements.
- `verification/resume-source.patch.gz`: safe three-way resumed source against bc90f8741dccbd062a94cc3e8abdae1c30d6296b, keeping62/63 legacy branches. `source-sha256.json` records current runtime/checker inputs.
- `verification/cargo-test.log`: final real-MQTT-broker workspace tests,220passed/3existing ignored/0failed, no skips. `cargo-test-parallel-failed.log` and `unknown-member-isolated.log` preserve the prior shared-broker parallel-test collision and successful serial diagnosis.
- `verification/code-review.md`: independent GPT-6.1 Sol high Standards and Spec review, corrected checker weaknesses and final strict mutation checks.
- `verification/evidence-sha256.json`: all retained evidence files except the manifest itself; verify after checkout to detect artifact changes.

To reproduce new current checks, build the two probe binaries, then freeze them and the driver/fixture/allocator into an independent runtime directory. Run `nat4_irregular_acceptance.py safety|compatibility PROBE QAD NEW_EVIDENCE_DIR` inside the dedicated privileged `miopunch-nat59-lab:bookworm` container from `scripts/nat4_irregular_lab.Dockerfile`. Never run from a binary path cargo tests may overwrite, reuse an evidence directory, inherit lab tunables, make the selector read fixture labels/seeds, or claim thread-RNG draws can be replayed from the fixture seed.

The original statistical runner likewise keeps AB/BA identity/allocator pairing and all misses. Its fixture is reproducible; actual random candidate sequences are retained evidence, not a deterministic statistical RNG contract. Raw successes are insufficient: original TLS/Connection/physical socket, approved IP path and authorized business must all be verified.
