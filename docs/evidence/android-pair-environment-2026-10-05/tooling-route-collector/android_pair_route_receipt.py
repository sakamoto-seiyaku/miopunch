#!/usr/bin/env python3
"""Read exact Android test actors' routing facts; never probe public services."""
import argparse
from datetime import datetime, timezone
import hashlib
import ipaddress
import json
from pathlib import Path
import re

from android_pair_runtime import ADB, Device


def utc():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def validate_request(value):
    if not isinstance(value, dict) or set(value) != {"endpoints"} or not isinstance(value["endpoints"], list) or not 1 <= len(value["endpoints"]) <= 2:
        raise ValueError("request must contain one or two explicit test endpoints")
    seen = set()
    for endpoint in value["endpoints"]:
        if not isinstance(endpoint, dict) or set(endpoint) - {"model", "actor", "selectedTuple", "socketEvidence"}:
            raise ValueError("unexpected endpoint field")
        model = endpoint.get("model")
        if model not in ("Pixel_6a", "TB322FC") or model in seen:
            raise ValueError("endpoint requires a unique exact supported model")
        seen.add(model)
        actor = endpoint.get("actor", {})
        if not isinstance(actor, dict):
            raise ValueError("explicit actor object required")
        if actor.get("kind") == "chrome":
            if actor != {"kind": "chrome", "package": "com.android.chrome"}:
                raise ValueError("only the exact Chrome package is readable")
        elif actor.get("kind") == "native":
            if set(actor) != {"kind", "ownedDirectory"} or not re.fullmatch(
                    r"/data/local/tmp/miopunch-pair-[0-9a-f]{32}", actor["ownedDirectory"]):
                raise ValueError("native UID requires the exact runtime-owned directory")
        else:
            raise ValueError("explicit Chrome or owned native actor required")
        selected = endpoint.get("selectedTuple", {})
        if not isinstance(selected, dict) or set(selected) != {"localAddress", "localPort", "peerAddress", "peerPort"}:
            raise ValueError("exact selected UDP tuple required")
        local, peer = (ipaddress.IPv4Address(selected[key]) for key in ("localAddress", "peerAddress"))
        if local.is_unspecified or local.is_multicast or local.is_loopback or peer.is_unspecified or peer.is_multicast or peer.is_loopback:
            raise ValueError("selected tuple must contain usable IPv4 addresses")
        for key in ("localPort", "peerPort"):
            if type(selected[key]) is not int or not 1 <= selected[key] <= 65535:
                raise ValueError("actual selected UDP ports required")
        evidence = endpoint.get("socketEvidence")
        if evidence is not None:
            if not isinstance(evidence, dict) or actor["kind"] != "native" or set(evidence) != {"operation", "success", "value", "pid", "localAddress", "localPort"}:
                raise ValueError("only a matching owned native getsockopt receipt is accepted")
            if (evidence["operation"] != "getsockopt(SO_MARK)" or evidence["success"] is not True
                    or type(evidence["value"]) is not int or not 0 <= evidence["value"] <= 0xffffffff
                    or type(evidence["pid"]) is not int or evidence["pid"] <= 0
                    or evidence["localAddress"] != selected["localAddress"]
                    or evidence["localPort"] != selected["localPort"]):
                raise ValueError("socket mark must be explicitly observed for this local tuple")
    return value


def collect_endpoint(endpoint, device):
    item = {"model": endpoint["model"], "actorKind": endpoint["actor"]["kind"],
            "selectedTuple": endpoint["selectedTuple"], "startedUtc": utc(), "observations": [],
            "errors": [], "socketMark": {"observed": False, "value": None},
            "actualSocketUid": {"observed": False, "value": None}}

    def read(name, *arguments):
        started = utc()
        stdout, code = device.shell(*arguments, required=False)
        item["observations"].append({"name": name, "startedUtc": started, "completedUtc": utc(),
                                     "exit": code, "stdout": stdout})
        cleanup = device.commands[-1].get("cleanup", {})
        command_clean = cleanup.get("forced") is False and cleanup.get("processGroupAbsentImmediately") is True
        if code or not command_clean:
            item["errors"].append({"operation": name, "exit": code, "localCommandCleanupObserved": command_clean})
        return stdout if code == 0 and command_clean else None

    addresses = read("physicalWifiIpv4", "ip", "-o", "-4", "addr", "show", "wlan0")
    read("ipv4PolicyRules", "ip", "-4", "rule", "show")
    read("allIpv4RouteTables", "ip", "-4", "route", "show", "table", "all")
    local = endpoint["selectedTuple"]["localAddress"]
    item["selectedLocalObservedOnWifi"] = addresses is not None and any(
        token.split("/", 1)[0] == local for token in addresses.split())
    if not item["selectedLocalObservedOnWifi"]:
        item["errors"].append({"operation": "selected local Wi-Fi address", "fact": "not observed"})
    uid = None
    actor = endpoint["actor"]
    if actor["kind"] == "chrome":
        output = read("exactChromePackageUid", "cmd", "package", "list", "packages", "-U", "com.android.chrome")
        matches = re.findall(r"^package:com\.android\.chrome\s+uid:(\d+)\s*$", output or "", re.MULTILINE)
        # Package-manager filters can match substrings. Retain only this exact
        # test package's facts, never a list of similarly named user packages.
        item["observations"][-1]["stdout"] = "".join(
            f"package:com.android.chrome uid:{value}\n" for value in matches)
        if len(matches) == 1:
            uid = int(matches[0])
            item["packageUid"] = uid
            item["routeUidProvenance"] = "observed Chrome package UID; network-service socket UID remains unknown"
    else:
        pid_file = actor["ownedDirectory"] + "/actor.pid"
        first = read("ownedPidBefore", "cat", pid_file)
        if first is not None and re.fullmatch(r"[1-9][0-9]*\s*", first):
            pid = int(first)
            status = read("ownedProcessUid", "sed", "-n", "/^Uid:/p", f"/proc/{pid}/status")
            second = read("ownedPidAfter", "cat", pid_file)
            matches = re.fullmatch(r"Uid:\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*", status or "")
            item["ownedPidUnchangedDuringUidRead"] = second is not None and second.strip() == str(pid)
            if matches and item["ownedPidUnchangedDuringUidRead"]:
                values = list(map(int, matches.groups()))
                item["ownedProcessUid"] = dict(zip(("real", "effective", "saved", "filesystem"), values))
                uid = values[1]
                item["routeUidProvenance"] = "observed owned native process effective UID; socket UID remains unknown"
                evidence = endpoint.get("socketEvidence")
                if evidence:
                    if evidence["pid"] == pid:
                        item["socketMark"] = {"observed": True, "value": evidence["value"],
                                              "provenance": "matching request's owned fixture getsockopt(SO_MARK) success receipt"}
                    else:
                        item["errors"].append({"operation": "socket mark ownership", "fact": "fixture PID differs from live owned PID"})
    if uid is None:
        item["errors"].append({"operation": "route lookup UID", "fact": "not observed; lookup skipped"})
        item["routeLookupSkipped"] = True
    elif item["selectedLocalObservedOnWifi"]:
        selected = endpoint["selectedTuple"]
        arguments = ["ip", "-4", "route", "get", selected["peerAddress"], "from", local, "uid", str(uid)]
        if item["socketMark"]["observed"]:
            arguments += ["mark", hex(item["socketMark"]["value"])]
        item["routeLookupUid"] = uid
        item["routeLookupIncludesObservedMark"] = item["socketMark"]["observed"]
        read("exactSelectedPeerRoute", *arguments)
    else:
        item["routeLookupSkipped"] = True
    item["commands"] = device.commands
    item["completedUtc"] = utc()
    item["physicalWanQualified"] = False
    item["limits"] = ["Kernel route lookup is read-only and emits no peer/STUN/TLS packet.",
                      "Chrome package UID and native process UID do not independently observe socket UID.",
                      "An omitted mark is a query limitation, never an observed socket mark of zero.",
                      "Only contemporaneous correlation with the actual SDK tuple and policy can support physical routing; no automatic qualification."]
    return item


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--request", type=Path, required=True, help="explicit actor and actual selected tuple; no secrets")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--adb", type=Path, default=ADB)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("receipt exists; preserve its original observation")
    request_bytes = args.request.read_bytes()
    try:
        request = validate_request(json.loads(request_bytes))
    except (ValueError, TypeError, KeyError) as error:
        parser.error(str(error))
    report = {"scope": "read-only actual test actor route corroboration; no SDK or public probe started",
              "startedUtc": utc(), "sourceSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "runtimeSha256": hashlib.sha256(Path(__file__).with_name("android_pair_runtime.py").read_bytes()).hexdigest(),
              "subprocessHelperSha256": hashlib.sha256(Path(__file__).with_name("wan_environment_preflight.py").read_bytes()).hexdigest(),
              "requestSha256": hashlib.sha256(request_bytes).hexdigest(), "endpoints": [],
              "physicalWanQualified": False, "networkSettingsChanged": False}
    for endpoint in request["endpoints"]:
        try:
            device = Device(endpoint["model"], "usb" if endpoint["model"] == "Pixel_6a" else "network", args.adb)
            item = collect_endpoint(endpoint, device)
        except Exception as error:
            item = {"model": endpoint["model"], "errors": [{"operation": "read-only collection", "failureClass": type(error).__name__}],
                    "physicalWanQualified": False}
        report["endpoints"].append(item)
    report["completedUtc"] = utc()
    report["commandExit"] = 2 if any(item["errors"] for item in report["endpoints"]) else 0
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(report, stream, indent=2); stream.write("\n")
    print(json.dumps({"receipt": str(args.output), "commandExit": report["commandExit"], "physicalWanQualified": False}))
    return report["commandExit"]


if __name__ == "__main__":
    raise SystemExit(main())
