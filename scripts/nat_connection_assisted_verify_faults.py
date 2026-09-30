#!/usr/bin/env python3
"""Reject six assisted/fallback corruptions using only unpacked scratch evidence."""
import argparse
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import tempfile

from nat_connection_assisted_verify import verify


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    base = parser.parse_args().evidence
    v = base / "verification"
    strict = v / "assisted-functional-strict.json"
    canonical = hashlib.sha256(strict.read_bytes()).hexdigest()
    faults = [
        ("missing-assisted", "assisted-with-s-blocked", lambda t: re.sub(r"(LEGACY_CANDIDATES .* assisted=)\[[^]]+\]", r"\g<1>[]", t, count=1)),
        ("mapped-before-assisted", "assisted-with-s-blocked", lambda t: re.sub(r"^(LEGACY_DATAGRAM target=10\.27\.1\.2:42027[^\n]+)\n(LEGACY_DATAGRAM target=100\.64\.27\.2:42027[^\n]+)$", r"\2\n\1", t, count=1, flags=re.M)),
        ("invented-no-S-mapped", "assisted-without-s-blocked", lambda t: t.replace("mapped=[]", "mapped=[100.64.27.6:43027]", 1)),
        ("no-S-low-TTL", "assisted-without-s-blocked", lambda t: t.replace("role: Sender, ttl: 0", "role: Sender, ttl: 7", 1)),
        ("no-S-disabled", "assisted-without-s-blocked", lambda t: t.replace("execution=Ready", "execution=Disabled", 1)),
        ("no-S-early-stop", "assisted-without-s-blocked", lambda t: re.sub(r"(LEGACY_PHASE_STOP .* elapsed_ms=)\d+", r"\g<1>100", t, count=1)),
    ]
    rows = []
    with tempfile.TemporaryDirectory(prefix="mio60-assisted-faults-") as tmp:
        scratch = pathlib.Path(tmp)
        shutil.copytree(v, scratch / "verification")
        for name, cell, transform in faults:
            shutil.copytree(base / "assisted-functional", scratch / "assisted-functional")
            for compressed in (scratch / "assisted-functional").rglob("*.gz"):
                with gzip.open(compressed, "rb") as source, compressed.with_suffix("").open("wb") as output:
                    shutil.copyfileobj(source, output)
                compressed.unlink()
            path = scratch / "assisted-functional" / cell / "server.log"
            before = path.read_text()
            after = transform(before)
            if before == after:
                raise RuntimeError("fault did not alter artifact: " + name)
            path.write_text(after)
            try:
                verify(scratch)
            except (ValueError, KeyError, IndexError) as error:
                rows.append(dict(fault=name, rejected=True, reason=str(error)))
            else:
                raise RuntimeError("fault accepted: " + name)
            shutil.rmtree(scratch / "assisted-functional")
    if hashlib.sha256(strict.read_bytes()).hexdigest() != canonical:
        raise RuntimeError("canonical supplemental evidence changed")
    result = dict(result="PASS", rejected=len(rows), canonical_strict_sha256=canonical, rows=rows)
    (v / "assisted-verifier-faults.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(dict(result="PASS", rejected=len(rows))))


if __name__ == "__main__":
    main()
