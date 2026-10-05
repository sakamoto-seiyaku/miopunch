#!/usr/bin/env python3
"""Bounded read-only WAN prerequisites; never declares SDK/physical-NAT success."""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import signal
import socket
import struct
import subprocess
import time


TRACE_URL = "https://www.cloudflare.com/cdn-cgi/trace"


def bounded_process(argv):
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, start_new_session=True)
    cleanup = {"forced": False, "leaderWaitedFor": False}
    try:
        stdout, stderr = process.communicate(timeout=12)
    except subprocess.TimeoutExpired:
        started = time.monotonic()
        cleanup.update(forced=True, reason="local command wait exceeded 12 seconds")
        try:
            os.killpg(process.pid, signal.SIGTERM)
            cleanup["termSent"] = True
        except ProcessLookupError:
            cleanup["termSent"] = False
        try:
            stdout, stderr = process.communicate(timeout=0.3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
                cleanup["killSent"] = True
            except ProcessLookupError:
                cleanup["killSent"] = False
            stdout, stderr = process.communicate()
        cleanup["elapsedMs"] = round((time.monotonic() - started) * 1000, 3)
    cleanup["leaderWaitedFor"] = process.returncode is not None
    try:
        os.killpg(process.pid, 0)
        cleanup["processGroupAbsentImmediately"] = False
    except ProcessLookupError:
        cleanup["processGroupAbsentImmediately"] = True
    return process.returncode, stdout, stderr, cleanup


def command(argv, *, display=None, transform=None):
    started = time.monotonic()
    record = {"command": ["rtk", "proxy", *(display or argv)]}
    try:
        returncode, stdout, stderr, cleanup = bounded_process(["rtk", "proxy", *argv])
        record.update(exit=returncode, stderr=stderr, cleanup=cleanup)
        if transform and returncode == 0:
            try:
                record["result"] = transform(stdout)
            except ValueError as error:
                record["parseFailure"] = str(error)
        elif transform:
            # Do not parse failed ip output or publish unfiltered partial dumps.
            record["filteredOutputOmittedOnFailure"] = True
        else:
            record["stdout"] = stdout
    except OSError as error:
        record.update(exit=None, failure=str(error))
    if display:
        # ADB errors can repeat the device identifier; replace it as in command.
        replacements = [(raw, shown) for raw, shown in zip(argv, display) if raw != shown]
        for field in ("stdout", "stderr", "failure"):
            if field in record:
                for raw, shown in replacements:
                    record[field] = record[field].replace(raw, shown)
    record["elapsedMs"] = round((time.monotonic() - started) * 1000, 3)
    return record


def trace(stdout):
    return {key: value for key, value in
            (line.split("=", 1) for line in stdout.splitlines() if "=" in line)
            if key in {"ip", "warp", "http", "tls"}}


def addresses(stdout):
    return [{"interface": item["ifname"], "flags": item.get("flags", []),
             "addresses": [{"family": value["family"], "local": value["local"],
                            "prefixlen": value["prefixlen"]}
                           for value in item.get("addr_info", [])]}
            for item in json.loads(stdout)]


def android_addresses(stdout):
    # Drop hardware addresses and lifetimes; retain the actual IP/interface facts.
    return [{"interface": match[1], "family": match[2], "address": match[3]}
            for line in stdout.splitlines()
            if (match := re.match(r"\d+:\s+(\S+)\s+(inet6?)\s+(\S+)", line))]


def android_networks(stdout):
    # Only current agent lines with interface names, not request/history entries.
    networks = []
    for line in stdout.splitlines():
        interface = re.search(r"InterfaceName:\s*(\S+)", line)
        transports = re.search(r"Transports:\s*([A-Z| ]+?)\s+Capabilities:", line)
        if interface and transports:
            networks.append({"interface": interface[1], "transports": transports[1]})
    return networks


def tls_command(bind):
    return ["curl", "--noproxy", "*", "--interface", bind,
            "--connect-timeout", "3", "--max-time", "6", "--silent",
            "--show-error", TRACE_URL]


def stun(bind, server):
    """Send exactly one IPv4 binding request, validating peer/header/transaction."""
    started = time.monotonic()
    cookie = 0x2112A442
    transaction = secrets.token_bytes(12)
    packet = struct.pack("!HHI", 1, 0, cookie) + transaction
    record = {"operation": "one raw STUN datagram", "bind": bind, "server": server,
              "scope": "UDP observation only; not RTC/SDK/path qualification"}
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.settimeout(3)
        sock.bind((bind, 0))
        record["source"] = list(sock.getsockname())
        sock.sendto(packet, tuple(server))
        response, peer = sock.recvfrom(2048)
        valid = (peer == tuple(server) and len(response) >= 20
                 and response[8:20] == transaction
                 and struct.unpack("!HHI", response[:8]) == (0x101, len(response) - 20, cookie))
        record.update(responsePeer=list(peer), responseBytes=len(response),
                      transactionMatched=valid)
        offset = 20
        while valid and offset + 4 <= len(response):
            kind, size = struct.unpack("!HH", response[offset:offset + 4])
            value = response[offset + 4:offset + 4 + size]
            if len(value) != size:
                raise ValueError("truncated STUN attribute")
            if kind == 0x20 and size == 8 and value[1] == 1:
                mapped_ip = socket.inet_ntoa(bytes(a ^ b for a, b in
                    zip(value[4:8], struct.pack("!I", cookie))))
                record["mapped"] = [mapped_ip, struct.unpack("!H", value[2:4])[0] ^ (cookie >> 16)]
            offset += 4 + ((size + 3) // 4) * 4
        record["exit"] = 0 if valid and "mapped" in record else 1
    except (OSError, ValueError) as error:
        record.update(exit=1, failure=str(error))
    finally:
        record["callerElapsedMs"] = round((time.monotonic() - started) * 1000, 3)
        cleanup_started = time.monotonic()
        sock.close()
        record.update(cleanupElapsedMs=round((time.monotonic() - cleanup_started) * 1000, 3),
                      ownedSocketClosed=sock.fileno() == -1)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new receipt file")
    parser.add_argument("--host-bind", type=ipaddress.IPv4Address)
    parser.add_argument("--adb", type=Path, default=Path.home() / "Android/Sdk/platform-tools/adb")
    parser.add_argument("--android-model", help="exact adb model; omits device serial in receipt")
    parser.add_argument("--android-bind", type=ipaddress.IPv4Address)
    parser.add_argument("--stun", help="optional numeric IPv4:port; one host UDP request")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("receipt already exists; preserve first failures in a new file")
    if args.android_bind and not args.android_model:
        parser.error("--android-bind requires --android-model")
    if args.stun and not args.host_bind:
        parser.error("--stun requires --host-bind")
    server = None
    if args.stun:
        try:
            address, port = args.stun.rsplit(":", 1)
            server = [str(ipaddress.IPv4Address(address)), int(port)]
            if not 1 <= server[1] <= 65535:
                raise ValueError("invalid port")
        except ValueError:
            parser.error("--stun requires numeric IPv4:port")
    source = Path(__file__).resolve()
    report = {"schema": 1, "scope": "read-only WAN prerequisites, not physical NAT or SDK pass",
              "physicalWanQualified": False, "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "scriptSha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "proxyEnvironmentNames": sorted(key for key in os.environ if "proxy" in key.lower()),
              "normalTlsVerification": True, "commands": []}
    commands = report["commands"]
    commands.append(command(["git", "rev-parse", "HEAD"]))
    commands.append(command(["ip", "-j", "address", "show"], transform=addresses))
    commands.append(command(["ip", "-j", "route", "show", "table", "all"], transform=json.loads))
    commands.append(command(["ip", "-j", "rule", "show"], transform=json.loads))
    if args.host_bind:
        commands.append(command(tls_command(str(args.host_bind)), transform=trace))
    if server:
        commands.append(command(["ip", "-j", "route", "get", server[0], "from", str(args.host_bind)], transform=json.loads))
        report["hostStun"] = stun(str(args.host_bind), server)
    if args.android_model:
        adb = str(args.adb)
        commands.append(command([adb, "version"]))
        matches = []
        def selected_devices(stdout):
            matches.extend(line.split()[0] for line in stdout.splitlines()
                           if "model:" + args.android_model in line.split()
                           and len(line.split()) > 1 and line.split()[1] == "device")
            return {"matchingDeviceCount": len(matches)}
        discovery = command([adb, "devices", "-l"], transform=selected_devices)
        commands.append(discovery)
        if len(matches) == 1:
            serial = matches[0]
            report["android"] = {"model": args.android_model,
                                 "controlTransport": "network" if ":" in serial else "usb"}
            for argv, transform in [(["ip", "-o", "addr", "show"], android_addresses),
                                    (["ip", "route", "show", "table", "all"], None),
                                    (["ip", "rule", "show"], None),
                                    (["dumpsys", "connectivity"], android_networks)]:
                remote = shlex.join(argv)
                commands.append(command([adb, "-s", serial, "shell", remote],
                    display=[adb, "-s", "<model-selected-device>", "shell", remote], transform=transform))
            if server:
                argv = ["ip", "route", "get", server[0]]
                remote = shlex.join(argv)
                commands.append(command([adb, "-s", serial, "shell", remote],
                    display=[adb, "-s", "<model-selected-device>", "shell", remote]))
            if args.android_bind:
                remote = shlex.join(tls_command(str(args.android_bind)))
                commands.append(command([adb, "-s", serial, "shell", remote],
                    display=[adb, "-s", "<model-selected-device>", "shell", remote], transform=trace))
        else:
            report["androidFailure"] = "expected exactly one connected device with the requested model"
    failures = [item for item in commands if item.get("exit") != 0 or "parseFailure" in item
                or item.get("cleanup", {}).get("forced")]
    report["firstCommandFailure"] = failures[0] if failures else None
    report["cleanup"] = {"requestedPersistentProcessesOrForwardings": False, "createdPeerConnections": False,
                         "localCommandLeadersWaitedFor": all(item.get("cleanup", {}).get("leaderWaitedFor", False)
                                                              for item in commands),
                         "hostUdpSocket": report.get("hostStun", {}).get("ownedSocketClosed")}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    print(json.dumps({"receipt": str(args.output), "commandFailures": len(failures),
                      "physicalWanQualified": False}))
    return 2 if failures or report.get("androidFailure") or report.get("hostStun", {}).get("exit", 0) else 0


if __name__ == "__main__":
    raise SystemExit(main())
