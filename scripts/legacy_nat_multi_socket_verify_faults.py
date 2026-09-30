#!/usr/bin/env python3
"""Reject deliberate corruptions of retained MIOPUNCH-61 evidence, offline."""
import argparse
import json
import os
import pathlib
import re
import shutil
import struct
import tempfile

from legacy_nat_multi_socket_matrix import cells, name
from legacy_nat_multi_socket_verify import EvidencePath, verify


def replace(directory, filename, transform):
    path = directory / filename
    value = transform(EvidencePath(path).read_bytes())
    path.unlink(missing_ok=True)  # Copied paths link to immutable originals.
    path.write_bytes(value)


def text_change(pattern, replacement):
    def change(value):
        result, count = re.subn(pattern, replacement, value.decode(), count=1, flags=re.M)
        if count != 1:
            raise RuntimeError("fault did not apply: " + pattern)
        return result.encode()
    return change


def first_raw(change_ttl=False):
    def change(value):
        value = bytearray(value)
        kind = struct.unpack("<I", value[20:24])[0]
        offset = 20 if kind == 276 else 14
        index = 24
        while index < len(value):
            sec, usec, size, _ = struct.unpack("<IIII", value[index:index + 16])
            ip = index + 16 + offset
            if value[ip] >> 4 == 4 and value[ip + 9] == 17:
                payload = ip + (value[ip] & 15) * 4 + 8
                if value[payload:payload + 11] == b"\0MIOPUNCH63" and value[payload + 27] == 0:
                    if change_ttl:
                        value[ip + 8] = 3
                    else:
                        value[index:index + 4] = struct.pack("<I", sec - 3)
                    return bytes(value)
            index += 16 + size
        raise RuntimeError("no captured raw request for fault")
    return change


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=pathlib.Path)
    args = parser.parse_args()
    repo = pathlib.Path(__file__).resolve().parents[1]
    oracle = json.loads((repo / "tests/fixtures/legacy_nat_behaviors.json").read_text())
    positive, corrected, miss, negative = cells()[0], cells()[12], cells()[18], cells()[34]
    faults = [
        ("capture_truncated", positive, "wan.pcap", lambda value: value[:-1]),
        ("capture_drops", positive, "tcpdump.log", text_change(r"^0 packets dropped by kernel$", "1 packets dropped by kernel")),
        ("physical_socket_changed", positive, "server.log", text_change(r"(LEGACY_DATAGRAM .*local_socket_id=)\d+", r"\g<1>999999")),
        ("wire_ttl_changed", positive, "lan-a.pcap", first_raw(change_ttl=True)),
        ("wire_send_before_delay", positive, "lan-a.pcap", first_raw()),
        ("primary_and_extra_merged", positive, "preflight-a-mapping.txt", lambda value: re.sub(rb"extra_source=\d+", re.search(rb"primary=(\d+)", value)[0].replace(b"primary", b"extra_source"), value)),
        ("listener_removed", positive, "server.log", text_change(r"^LEGACY_LISTENER .*primary=false\n", "")),
        ("winner_business_socket_changed", positive, "server.log", text_change(r"(LEGACY_BUSINESS_BINDING .*local_socket_id=)\d+", r"\g<1>0")),
        ("winner_ready_reset_budget", corrected, "client.log", text_change(r"(LEGACY_WINNER_READY .*remaining_ms=)\d+", r"\g<1>38001")),
        ("bounded_miss_stopped_early", miss, "client.log", text_change(r"(LEGACY_PHASE_STOP .*elapsed_ms=)\d+", r"\g<1>30000")),
        ("bounded_miss_control_lost", miss, "client.log", text_change(r"^(EARLY_FAILURE_CONTROL_ALIVE|CONTROL_ALIVE_AFTER_UDP_FAILURE) stable_id=\d+\n", "")),
        ("random_iterations_changed", miss, "client.log", text_change(r"(LEGACY_TRACE .*random_iterations=)1000", r"\g<1>999")),
        ("raw_work_before_admission", negative, "server.log", lambda value: value + b"LEGACY_DATAGRAM target=100.64.27.2:42027 ttl=0 response=false degraded=false local_socket_id=0\n"),
        ("wrong_rejected_identity", negative, "server.log", text_change(r"(ADMISSION_REJECTED peer=)[0-9a-f]{64}", r"\g<1>" + "f" * 64)),
        ("run_exit_changed", negative, "run.txt", text_change(r"client_rc=1", "client_rc=0")),
        ("extra_sources_bypass_allocator", positive, "fixture.sh", text_change(r'-p udp -d "\$peer_ip" -j NFQUEUE --queue-num 61', '-p udp --sport "$source" -d "$peer_ip" -j NFQUEUE --queue-num 61')),
    ]
    rows = []
    with tempfile.TemporaryDirectory(prefix="mio61-evidence-faults-") as temp:
        for label, cell, filename, mutation in faults:
            directory = pathlib.Path(temp) / label
            source = args.evidence / "canonical" / name(cell)
            shutil.copytree(source, directory, copy_function=lambda src, dst: os.symlink(os.path.abspath(src), dst))
            replace(directory, filename, mutation)
            try:
                verify(directory, cell, oracle)
            except ValueError as error:
                rows.append(dict(fault=label, cell=name(cell), result="REJECTED", reason=str(error).replace(temp, "<fault-root>")))
            else:
                raise RuntimeError("strict verifier accepted fault: " + label)
    print(json.dumps(dict(result="PASS", rejected=len(rows), rows=rows), indent=2))


if __name__ == "__main__":
    main()
