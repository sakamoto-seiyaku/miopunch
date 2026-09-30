#!/usr/bin/env python3
"""Adapt the retained NAT-57 kernel fixture for the explicitly partial 62 entry.

Fixture topology stays out of the product process environment. Raw evidence and
the exact adapted shell script are retained; a fixture failure is never accepted
as a traversal miss. Full range/TTL/listener execution belongs to later tickets.
"""
import argparse
import os
import pathlib
import subprocess
import tempfile


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise RuntimeError(f"retained fixture changed at anchor: {old[:70]!r}")
    return source.replace(old, new, 1)


def adapted_fixture(repo, topology, dialer):
    source = (repo / "scripts/nat4_lab.sh").read_text()
    source = replace_once(source, "strategy=${MIOPUNCH_NAT_STRATEGY:-baseline}", "strategy=legacy")
    source = replace_once(source, "topology=${MIOPUNCH_NAT_TOPOLOGY:-up1}", f"topology={topology}")
    source = replace_once(source, "dialer=${MIOPUNCH_NAT_DIALER:-a}", f"dialer={dialer}")
    source = replace_once(source, "baseline|auto)", "legacy)")
    source = replace_once(source, "up1|down1|up2|down2|eim)", "up1|down1|up2|down2|eim|nonmonotone|ipchange)")
    source = replace_once(source, "  eim) base=42027; step=0 ;;", "  eim) base=42027; step=0 ;;\n  nonmonotone) base=42031; step=1 ;;\n  ipchange) base=42031; step=0 ;;")
    source = replace_once(source, "  mapped_port=$((base + index * step))", '''  mapped_port=$((base + index * step))
  mapped_ip=100.64.27.2
  if [[ $topology == nonmonotone ]]; then
    observed_ports=(42031 42033 42032 42031)
    mapped_port=${observed_ports[$index]}
  fi
  if [[ $topology == ipchange && $((index % 2)) -eq 1 ]]; then mapped_ip=100.64.27.10; fi''')
    source = replace_once(source, '"100.64.27.2:$mapped_port"', '"$mapped_ip:$mapped_port"')
    source = replace_once(source, "for index in 0 1 2 3; do", '''if [[ $topology == ipchange ]]; then
  ip -n "$nat_a" addr add 100.64.27.10/32 dev wan0
  ip -n "$wan" route add 100.64.27.10/32 via 100.64.27.2
fi
for index in 0 1 2 3; do''')
    source = replace_once(source, "if [[ $strategy == auto ]]; then", "if [[ $strategy == legacy && ${MIOPUNCH_LEGACY_FALLBACK:-0} != 1 ]]; then")
    start = source.index('case "$case_name" in\n  positive)')
    source = source[:start] + '''case "$case_name" in
  positive)
    grep -q LEGACY_SELECTED "$evidence/client.log"
    grep -q LEGACY_SELECTED "$evidence/server.log"
    if [[ $topology == eim ]]; then
      grep -q DATA_OK "$evidence/client.log"
      grep -q DATA_OK "$evidence/server.log"
      grep -q 'LEGACY_RESULT outcome=confirmed_business' "$evidence/server.log"
    elif [[ $topology == ipchange || $topology == up2 || $topology == down2 ]]; then
      grep -q ExecutionUnavailable "$evidence/client.log"
      grep -q ExecutionUnavailable "$evidence/server.log"
      ! grep -q PROBES_SENT "$evidence/client.log"
      ! grep -q PROBES_SENT "$evidence/server.log"
    else
      grep -q PATH_UNAVAILABLE "$evidence/client.log"
      grep -q PATH_UNAVAILABLE "$evidence/server.log"
      ! grep -q DATA_OK "$evidence/client.log"
      ! grep -q DATA_OK "$evidence/server.log"
    fi ;;
  blocked|wrong)
    grep -q PATH_UNAVAILABLE "$evidence/client.log"
    grep -q PATH_UNAVAILABLE "$evidence/server.log"
    ! grep -q DATA_OK "$evidence/client.log"
    ! grep -q DATA_OK "$evidence/server.log" ;;
  stale)
    grep -q STALE_CANDIDATE "$evidence/client.log"
    grep -q COORD_UNAVAILABLE "$evidence/server.log" ;;
  nonmember|revoked|no_secret|wrong_secret)
    grep -Eq 'ADMISSION_REJECTED|Error:' "$evidence/client.log"
    grep -Eq 'ADMISSION_REJECTED|Error:' "$evidence/server.log"
    ! grep -q LEGACY_SAMPLE "$evidence/client.log"
    ! grep -q LEGACY_SAMPLE "$evidence/server.log"
    ! grep -q LEGACY_SELECTED "$evidence/client.log"
    ! grep -q LEGACY_SELECTED "$evidence/server.log"
    ! grep -q PROBES_SENT "$evidence/client.log"
    ! grep -q PROBES_SENT "$evidence/server.log" ;;
  *) exit 2 ;;
esac
'''
    return source


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("case", choices=["positive", "blocked", "wrong", "stale", "nonmember", "revoked", "no_secret", "wrong_secret"])
    parser.add_argument("topology", choices=["eim", "up1", "down1", "up2", "down2", "nonmonotone", "ipchange"])
    parser.add_argument("dialer", choices=["a", "b"])
    parser.add_argument("probe", type=pathlib.Path)
    parser.add_argument("qad", type=pathlib.Path)
    parser.add_argument("evidence", type=pathlib.Path)
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("run in the isolated privileged NAT lab container")
    repo = pathlib.Path(__file__).resolve().parents[1]
    source = adapted_fixture(repo, args.topology, args.dialer)
    args.evidence.mkdir(parents=True, exist_ok=True)
    (args.evidence / "fixture.sh").write_text(source)
    environment = dict(os.environ)
    for name in ["MIOPUNCH_NAT_TOPOLOGY", "MIOPUNCH_NAT_DIALER", "MIOPUNCH_NAT_STRATEGY"]:
        environment.pop(name, None)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".sh", prefix="miopunch62-fixture-") as script:
        script.write(source)
        script.flush()
        result = subprocess.run(["bash", script.name, args.case, str(args.probe), str(args.qad), str(args.evidence)], env=environment)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
