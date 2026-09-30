#!/usr/bin/env python3
"""Deliberate corruptions must fail independently; canonical evidence is untouched."""
import argparse
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import tempfile
from nat_connection_verify import verify


def edit(directory, name, transform):
    path = directory / name
    before = path.read_text()
    after = transform(before)
    if before == after:
        raise RuntimeError("fault did not alter selected artifact: " + name)
    path.write_text(after)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    args = parser.parse_args()
    v = args.evidence / "verification"
    plans = {1: json.loads((v / "frozen-plan.json").read_text()), 2: json.loads((v / "frozen-plan-generation2.json").read_text())}
    versions = json.loads((v / "runtime-by-cell.json").read_text())
    rows = json.loads((v / "strict.json").read_text())["comparison"]
    positive = next(row["cell"] for row in rows if row["kind"] == "punching" and row["outcome"] == "confirmed_business" and row["phases"][0]["mode"] == 2)
    missed = next(row["cell"] for row in rows if row["kind"] == "punching" and row["outcome"] == "bounded_miss")
    cases = {cell["id"]: cell for cell in plans[1]["comparison"]}

    def corrupt_mapping(d):
        def transform(text):
            lines = text.splitlines()
            reset = lines.index("ALLOCATOR_RESET")
            for i in range(reset + 1, len(lines)):
                if lines[i].startswith("{"):
                    row = json.loads(lines[i])
                    if "mapped" in row:
                        row["mapped"][1] += 1
                        lines[i] = json.dumps(row)
                        return "\n".join(lines) + "\n"
            raise RuntimeError("no product tuple for fault")
        edit(d, "allocator-a.log", transform)

    faults = [
        ("missing-S", positive, lambda d: edit(d, "server.log", lambda t: re.sub(r"^LEGACY_SAMPLE order=0 .+\n", "", t, count=1, flags=re.M))),
        ("wrong-mode", positive, lambda d: edit(d, "server.log", lambda t: t.replace("LEGACY_SELECTED mode=2", "LEGACY_SELECTED mode=4", 1))),
        ("wrong-score", positive, lambda d: edit(d, "server.log", lambda t: t.replace("score: 0", "score: 9", 1))),
        ("wrong-physical-source", positive, corrupt_mapping),
        ("wrong-business-socket", positive, lambda d: edit(d, "server.log", lambda t: re.sub(r"(LEGACY_BUSINESS_BINDING .* local_socket_id=)(\d+)", lambda m: m[1] + str(int(m[2]) + 1), t, count=1))),
        ("wrong-TLS-peer", positive, lambda d: edit(d, "server.log", lambda t: re.sub(r"(NODE_SESSION local=[0-9a-f]+ peer=)[0-9a-f]+", r"\g<1>" + "1" * 64, t, count=1))),
        ("wrong-winner-path", positive, lambda d: edit(d, "server.log", lambda t: re.sub(r"(LEGACY_BUSINESS_BINDING .* path_id=)(\d+)", lambda m: m[1] + str(int(m[2]) + 1), t, count=1))),
        ("CPU-parent-scope", positive, lambda d: edit(d, "client-cpu.json", lambda t: t.replace("actual native node process", "docker client process"))),
        ("missing-CPU", positive, lambda d: edit(d, "server-cpu.json", lambda t: json.dumps({k: val for k, val in json.loads(t).items() if k != "user_cpu_seconds"}))),
        ("changed-runtime", positive, lambda d: edit(d, "client-cpu.json", lambda t: json.dumps(dict(json.loads(t), executable_sha256="0" * 64)))),
        ("dropped-capture", positive, lambda d: edit(d, "tcpdump-lan-a.log", lambda t: t.replace("0 packets dropped by kernel", "1 packets dropped by kernel"))),
        ("truncated-capture", positive, lambda d: (d / "wan.pcap").write_bytes((d / "wan.pcap").read_bytes()[:-10])),
        ("early-bounded-miss", missed, lambda d: edit(d, "server.log", lambda t: re.sub(r"(LEGACY_PHASE_STOP .* elapsed_ms=)\d+", r"\g<1>100", t, count=1))),
        ("lost-original-control", missed, lambda d: edit(d, "client.log", lambda t: t.replace("EARLY_FAILURE_CONTROL_ALIVE", "CONTROL_LIVENESS_FAILED"))),
        ("listener-attempts-shrunk", positive, lambda d: edit(d, "server.log", lambda t: t.replace("listener_attempted=256", "listener_attempted=16", 1))),
        ("wrong-seed", positive, lambda d: edit(d, "run.txt", lambda t: re.sub(r"seed=\S+", "seed=chased-seed", t))),
    ]
    receipts = []
    before = hashlib.sha256((v / "strict.json").read_bytes()).hexdigest()
    with tempfile.TemporaryDirectory(prefix="mio60-verifier-faults-") as scratch:
        for name, cell_id, mutate in faults:
            target = pathlib.Path(scratch) / name
            shutil.copytree(args.evidence / "comparison" / cell_id, target)
            # Mutate only unpacked scratch copies; canonical gzip streams and
            # their round-trip/hash receipt remain unchanged.
            for compressed in target.rglob("*.gz"):
                with gzip.open(compressed, "rb") as source, compressed.with_suffix("").open("wb") as output:
                    shutil.copyfileobj(source, output)
                compressed.unlink()
            mutate(target)
            try:
                verify(target, cases[cell_id], plans[versions[cell_id]], versions[cell_id])
            except (ValueError, KeyError, IndexError, StopIteration) as error:
                receipts.append(dict(fault=name, cell=cell_id, rejected=True, reason=str(error)))
            else:
                raise RuntimeError("corrupt evidence accepted: " + name)
    if hashlib.sha256((v / "strict.json").read_bytes()).hexdigest() != before:
        raise RuntimeError("canonical strict output changed during fault checks")
    (v / "verifier-faults.json").write_text(json.dumps(dict(result="PASS", canonical_strict_sha256=before, rejected=len(receipts), rows=receipts), indent=2) + "\n")
    print(f"PASS {len(receipts)} independent evidence corruptions rejected")


if __name__ == "__main__":
    main()
