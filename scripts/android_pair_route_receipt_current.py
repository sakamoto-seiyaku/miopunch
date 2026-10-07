#!/usr/bin/env python3
"""Collect private, read-only route corroboration for the current Android pair.

The collector reads the observed device/network state and asks the kernel for
the route to the exact selected peer tuple.  It never sends a peer, STUN, TLS,
or SDK packet.  Its stdout contains only counts and qualification booleans;
raw addresses, proc rows, and SDK/PC observations remain in a mode-0600
receipt.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import socket
import struct

try:
    from android_pair_runtime_current import ADB, CurrentDevice, CURRENT_DEVICES
    from android_pair_packet_witness import correlate_selected_tuples
except ModuleNotFoundError:  # ``python -m scripts...`` from the repository root.
    from scripts.android_pair_runtime_current import ADB, CurrentDevice, CURRENT_DEVICES
    from scripts.android_pair_packet_witness import correlate_selected_tuples

ROOT = Path(__file__).resolve().parents[1]
NATIVE_MODEL = CURRENT_DEVICES["native"]["model"]
BROWSER_MODEL = CURRENT_DEVICES["browser"]["model"]
_DIRECTORY_RE = re.compile(r"/data/local/tmp/miopunch-pair-[0-9a-f]{32}\Z")


def utc() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _fail(message: str):
    raise ValueError(message)


def _tuple(value: object, *, allow_ipv6: bool = True) -> dict:
    if not isinstance(value, dict) or set(value) != {
            "localAddress", "localPort", "peerAddress", "peerPort"}:
        _fail("exact selected local/peer tuple required")
    try:
        local = ipaddress.ip_address(value["localAddress"])
        peer = ipaddress.ip_address(value["peerAddress"])
    except ValueError as error:
        raise ValueError("selected tuple contains an invalid address") from error
    if local.version != peer.version or (local.version == 6 and not allow_ipv6):
        _fail("selected tuple address family is unsupported or mixed")
    if any(address.is_unspecified or address.is_loopback or address.is_multicast
           for address in (local, peer)):
        _fail("selected tuple must contain usable non-loopback addresses")
    for key in ("localPort", "peerPort"):
        if type(value[key]) is not int or not 1 <= value[key] <= 65535:
            _fail("selected tuple ports must be integers in the UDP range")
    return {"localAddress": str(local), "localPort": value["localPort"],
            "peerAddress": str(peer), "peerPort": value["peerPort"]}


def _bool(value: object, field: str) -> bool:
    if type(value) is not bool:
        _fail(f"{field} must be an explicit boolean")
    return value


def _nonnegative_int(value: object, field: str) -> int:
    if type(value) is not int or value < 0:
        _fail(f"{field} must be a non-negative integer")
    return value


def _current_member(value: object, selected: dict) -> dict:
    if not isinstance(value, dict) or set(value) != {
            "sameOriginal", "alive", "memberGate", "selectedRtc", "localTuple"}:
        _fail("current live ordinary-M witness is incomplete")
    result = {key: _bool(value[key], key) for key in
              ("sameOriginal", "alive", "memberGate", "selectedRtc")}
    if not all(result.values()):
        _fail("current ordinary-M witness must prove same live M, member gate, and selected RTC")
    result["localTuple"] = _tuple(value["localTuple"])
    if result["localTuple"] != selected:
        _fail("current ordinary-M tuple does not match the selected tuple")
    return result


def _socket_evidence(value: object, selected: dict, actor_kind: str) -> dict | None:
    if value is None:
        return None
    if actor_kind != "native" or not isinstance(value, dict):
        _fail("socket mark evidence is only accepted for the owned Native actor")
    allowed = {"observed", "value", "socketEvidence", "matchingFdCount", "duplicatesClosed"}
    if set(value) - allowed:
        _fail("unexpected socket evidence field")
    observed = _bool(value.get("observed"), "socket mark observed")
    if not observed and value.get("value") is not None:
        _fail("unobserved socket mark cannot carry a guessed value")
    if type(value.get("matchingFdCount")) is int and value["matchingFdCount"] < 0:
        _fail("matching socket FD count is invalid")
    if observed:
        evidence = value.get("socketEvidence")
        if not isinstance(evidence, dict) or set(evidence) != {
                "operation", "success", "value", "pid", "localAddress", "localPort", "inode"}:
            _fail("observed socket mark requires the exact private observer ABI")
        if (evidence["operation"] != "getsockopt(SO_MARK)" or evidence["success"] is not True
                or type(evidence["value"]) is not int or not 0 <= evidence["value"] <= 0xFFFFFFFF
                or type(evidence["pid"]) is not int or evidence["pid"] <= 0
                or type(evidence["inode"]) is not int or evidence["inode"] <= 0
                or evidence["localAddress"] != selected["localAddress"]
                or evidence["localPort"] != selected["localPort"]):
            _fail("socket mark observation is not correlated to the selected local tuple")
        return {"observed": True, "value": evidence["value"],
                "pid": evidence["pid"], "inode": evidence["inode"],
                "localAddress": evidence["localAddress"], "localPort": evidence["localPort"],
                "operation": evidence["operation"]}
    # A missing observer result is unknown, including mark zero.  It is never
    # promoted to an observed default.
    return {"observed": False, "value": None}


def _pc_stats(value: object, selected: dict) -> dict:
    if not isinstance(value, dict) or set(value) != {
            "source", "selected", "sameOriginal", "alive", "memberGate",
            "localTuple", "peerTuple", "pingCount", "pongCount"}:
        _fail("browser-owned selected-PC stats are incomplete")
    if value["source"] != "browser-owned-selected-pc":
        _fail("CDP or browser WebSocket data cannot substitute for selected-PC stats")
    for field in ("selected", "sameOriginal", "alive", "memberGate"):
        _bool(value[field], field)
    if not all(value[field] is True for field in ("selected", "sameOriginal", "alive", "memberGate")):
        _fail("browser selected-PC witness must prove the same live M and member gate")
    local = _tuple(value["localTuple"])
    peer = _tuple(value["peerTuple"])
    if local != selected:
        _fail("browser selected-PC local tuple does not match the selected tuple")
    if peer["peerAddress"] != selected["peerAddress"] or peer["peerPort"] != selected["peerPort"]:
        _fail("browser selected-PC peer tuple does not match the selected tuple")
    return {"source": value["source"], "selected": value["selected"],
            "sameOriginal": value["sameOriginal"], "alive": value["alive"],
            "memberGate": value["memberGate"], "pingCount": _nonnegative_int(value["pingCount"], "pingCount"),
            "pongCount": _nonnegative_int(value["pongCount"], "pongCount"),
            "localTuple": local, "peerTuple": peer}


def _candidate(value: object, field: str, require_base: bool) -> dict:
    """Normalize one selected ICE candidate without losing its wire/base facts."""

    if not isinstance(value, dict):
        _fail(f"{field} candidate is incomplete")
    address = value.get("address") or value.get("ip")
    port = value.get("port")
    candidate_type = value.get("candidateType", value.get("typ", value.get("type")))
    protocol = value.get("protocol")
    if not isinstance(address, str) or type(port) is not int or not 1 <= port <= 65535:
        _fail(f"{field} candidate address/port is invalid")
    try:
        parsed = ipaddress.ip_address(address)
    except ValueError as error:
        raise ValueError(f"{field} candidate address is invalid") from error
    if parsed.is_unspecified or parsed.is_loopback or parsed.is_multicast:
        _fail(f"{field} candidate address is not usable")
    if candidate_type not in {"host", "srflx", "prflx", "relay"}:
        _fail(f"{field} candidate type is missing or unknown")
    if protocol is not None and protocol.lower() != "udp":
        _fail(f"{field} selected candidate is not UDP")
    related_address = value.get("relatedAddress", value.get("related_address"))
    related_port = value.get("relatedPort", value.get("related_port"))
    if related_address in (None, "") and related_port in (None, 0):
        related_address, related_port = None, None
    if related_address is not None or related_port is not None:
        if not isinstance(related_address, str) or type(related_port) is not int \
                or not 1 <= related_port <= 65535:
            _fail(f"{field} candidate base address/port is invalid")
        try:
            related = ipaddress.ip_address(related_address)
        except ValueError as error:
            raise ValueError(f"{field} candidate base address is invalid") from error
        if related.is_unspecified or related.is_loopback or related.is_multicast:
            _fail(f"{field} candidate base address is not usable")
    # srflx/prflx addresses are the nominated wire address.  The actual local
    # socket is the related host candidate; accepting a mapped address as the
    # local route would silently turn an overlay/public mapping into proof of
    # physical egress.
    if candidate_type in {"srflx", "prflx"}:
        if not isinstance(related_address, str) or type(related_port) is not int:
            if require_base:
                _fail(f"{field} mapped candidate lacks a proven host base")
            base_address, base_port = address, port
        else:
            base_address, base_port = related_address, related_port
    else:
        base_address, base_port = address, port
    return {"address": str(parsed), "port": port, "candidateType": candidate_type,
            "protocol": protocol.lower() if isinstance(protocol, str) else None,
            "relatedAddress": related_address, "relatedPort": related_port,
            "baseAddress": base_address, "basePort": base_port}


def _selected_pair(value: object, field: str = "selected") -> dict:
    """Validate selected local/remote candidates and preserve nonrelay facts."""

    if not isinstance(value, dict):
        _fail(f"{field} selected pair is incomplete")
    local = _candidate(value.get("local"), f"{field}.local", True)
    remote = _candidate(value.get("remote"), f"{field}.remote", False)
    if local["candidateType"] == "relay" or remote["candidateType"] == "relay":
        _fail(f"{field} selected pair uses a relay candidate")
    return {"local": local, "remote": remote,
            "nonRelay": True,
            "wireCorrelation": {"local": {
                "address": local["address"], "port": local["port"]},
                "remote": {"address": remote["address"], "port": remote["port"]}}}


def validate_request(value: object) -> dict:
    if not isinstance(value, dict) or set(value) - {"endpoints", "packetWitness"} \
            or not isinstance(value.get("endpoints"), list):
        _fail("request requires endpoints and an optional exact packet witness")
    if len(value["endpoints"]) != 2:
        _fail("current route receipt requires exactly the observed Native and Browser pair")
    roles = set()
    models = set()
    validated = []
    for endpoint in value["endpoints"]:
        if not isinstance(endpoint, dict) or set(endpoint) - {
                "role", "model", "transport", "actor", "selectedTuple", "currentM",
                "socketEvidence", "pcStats", "selectedPair"}:
            _fail("unexpected endpoint field")
        role, model, transport = endpoint.get("role"), endpoint.get("model"), endpoint.get("transport")
        if role not in CURRENT_DEVICES or role in roles or model in models:
            _fail("endpoint roles/models must be unique current pair entries")
        expected = CURRENT_DEVICES[role]
        if model != expected["model"] or transport != "usb":
            _fail("current pair endpoints must be exact observed USB devices")
        roles.add(role); models.add(model)
        actor = endpoint.get("actor")
        if not isinstance(actor, dict) or set(actor) - {"kind", "ownedDirectory", "package"}:
            _fail("explicit actor provenance is required")
        if role == "native":
            if actor.get("kind") != "native" or set(actor) != {"kind", "ownedDirectory"}:
                _fail("Native endpoint requires its exact owned directory")
            if not isinstance(actor["ownedDirectory"], str) or not _DIRECTORY_RE.fullmatch(actor["ownedDirectory"]):
                _fail("Native owned directory is not an exact runtime-owned path")
            if "pcStats" in endpoint:
                _fail("Native endpoint cannot claim browser PC stats")
        else:
            if actor != {"kind": "chrome", "package": "com.android.chrome"}:
                _fail("Browser endpoint requires the exact Chrome package")
            if "socketEvidence" in endpoint:
                _fail("Browser package/Chrome evidence cannot claim a Native socket mark")
        selected = _tuple(endpoint.get("selectedTuple"))
        current = _current_member(endpoint.get("currentM"), selected)
        evidence = _socket_evidence(endpoint.get("socketEvidence"), selected, actor["kind"])
        pc = _pc_stats(endpoint.get("pcStats"), selected) if role == "browser" else None
        pair = _selected_pair(endpoint.get("selectedPair"), f"{role}.selectedPair") \
            if endpoint.get("selectedPair") is not None else None
        if pair is not None:
            expected_local = {"localAddress": pair["local"]["baseAddress"],
                              "localPort": pair["local"]["basePort"],
                              "peerAddress": pair["remote"]["address"],
                              "peerPort": pair["remote"]["port"]}
            if expected_local != selected:
                _fail(f"{role} selected tuple does not match the candidate base/wire pair")
        validated.append({"role": role, "model": model, "transport": transport, "actor": actor,
                          "selectedTuple": selected, "currentM": current,
                          "socketEvidence": evidence, "pcStats": pc,
                          "selectedPair": pair})
    if roles != set(CURRENT_DEVICES) or models != {NATIVE_MODEL, BROWSER_MODEL}:
        _fail("both current endpoint roles are required")
    by_role = {item["role"]: item for item in validated}
    native_pair, browser_pair = by_role["native"].get("selectedPair"), by_role["browser"].get("selectedPair")
    pair_correlation = False
    if native_pair is not None and browser_pair is not None:
        pair_correlation = (
            native_pair["local"]["address"] == browser_pair["remote"]["address"]
            and native_pair["local"]["port"] == browser_pair["remote"]["port"]
            and native_pair["remote"]["address"] == browser_pair["local"]["address"]
            and native_pair["remote"]["port"] == browser_pair["local"]["port"])
    packet_correlation = None
    if "packetWitness" in value:
        if native_pair is None or browser_pair is None:
            _fail("packet correlation requires both actual selected ICE pairs")
        packet_correlation = correlate_selected_tuples(
            value["packetWitness"], by_role["native"]["selectedTuple"],
            by_role["browser"]["selectedTuple"])
        pair_correlation = packet_correlation["correlated"]
    return {"endpoints": validated, "pairCorrelation": pair_correlation,
            "packetCorrelation": packet_correlation}


def _sanitize_raw(text: str) -> str:
    # Keep raw route/address observations private while avoiding obvious secret
    # material if an Android diagnostic unexpectedly includes it.
    return "\n".join(line for line in text.splitlines()
                   if not re.search(r"(?i)(password|private.?key|pre.?shared.?key|auth.?token|secret)\s*[:=]", line)) + "\n"


def _agent_facts(connectivity: str) -> dict:
    active_match = re.search(r"Active default network:\s*(\d+)", connectivity, re.IGNORECASE)
    active = active_match.group(1) if active_match else None
    section = connectivity.split("Current Networks:", 1)[1].split("Network Requests:", 1)[0] \
        if "Current Networks:" in connectivity else ""
    # dumpsys connectivity wraps each NetworkAgentInfo over multiple lines;
    # parse records rather than only the header.  An active ID is required to
    # select a record, so a generic WIFI/VPN line cannot become proof.
    records = [record for record in re.split(r"(?=NetworkAgentInfo\{)", section)
               if record.lstrip().startswith("NetworkAgentInfo{")]
    selected = [record for record in records
                if active and re.search(r"(?:netId=|networkId=|\b)" + re.escape(active) + r"\b", record)]
    joined = "\n".join(selected)
    transport_match = re.search(r"Transports:\s*(.*?)\s+Capabilities:", joined)
    capability_match = re.search(r"Capabilities:\s*(.*?)(?:\s+LinkProperties:|\s+NetworkSpecifier:|$)", joined)
    transports = transport_match.group(1).split() if transport_match else []
    if not transports:
        transports = re.findall(r"\b(?:WIFI|CELLULAR|VPN)\b", joined)
    capability_text = capability_match.group(1) if capability_match else ""
    if not capability_text:
        capability_text = joined
    capabilities = re.split(r"[\s&,|]+", capability_text)
    interfaces = sorted(set(re.findall(
        r"(?:InterfaceName|Iface|interfaceName):\s*([^\s,}\]]+)", joined,
        re.IGNORECASE)))
    vpn_agents = 0
    for agent in records:
        match = re.search(r"Transports:\s*(.*?)\s+Capabilities:", agent)
        if match and "VPN" in match.group(1).split():
            vpn_agents += 1
    return {
        "activeDefaultObserved": active is not None,
        "defaultAgentObserved": bool(selected),
        "defaultValidated": "VALIDATED" in capabilities or bool(re.search(r"\bVALIDATED\b", capability_text)),
        "defaultNotVpnCapability": "NOT_VPN" in capabilities or bool(re.search(r"\bNOT_VPN\b", capability_text)),
        "defaultTransportCellular": "CELLULAR" in transports,
        "defaultTransportWifi": "WIFI" in transports,
        "defaultTransport": ("WIFI" if "WIFI" in transports else
                              "CELLULAR" if "CELLULAR" in transports else None),
        "defaultInterfaceCount": len(interfaces),
        "currentVpnAgentCount": vpn_agents,
        "interfaces": interfaces,
        "selectedAgent": joined,
    }


def _address_facts(address_text: str, interfaces: list[str]) -> dict:
    try:
        addresses = json.loads(address_text)
    except ValueError:
        addresses = []
    if not isinstance(addresses, list):
        addresses = []
    selected = [row for row in addresses if isinstance(row, dict) and row.get("ifname") in interfaces]
    tunnel = [row for row in addresses if isinstance(row, dict)
              and re.match(r"^(tun|tap|wg|warp|tailscale)", str(row.get("ifname", "")), re.I)]
    return {
        "defaultInterfaceIpv4Count": sum(1 for row in selected for item in row.get("addr_info", [])
                                          if isinstance(item, dict) and item.get("family") == "inet"),
        "defaultInterfaceGlobalIpv6Count": sum(1 for row in selected for item in row.get("addr_info", [])
                                                if isinstance(item, dict) and item.get("family") == "inet6"
                                                and item.get("scope") == "global"),
        "tunnelInterfaceCount": len(tunnel),
        "upTunnelInterfaceCount": sum("UP" in row.get("flags", []) for row in tunnel),
        "addressedTunnelInterfaceCount": sum(bool(row.get("addr_info")) for row in tunnel),
        "addresses": addresses,
    }


def _decode_proc_ipv4(value: str) -> str | None:
    try:
        packed = struct.pack("<I", int(value, 16))
        return socket.inet_ntoa(packed)
    except (ValueError, OSError, struct.error):
        return None


def _proc_rows(text: str, selected: dict) -> list[dict]:
    if ":" not in selected["localAddress"] or ":" not in selected["peerAddress"]:
        rows = []
        for line in text.splitlines()[1:]:
            fields = line.split()
            if len(fields) < 10 or ":" not in fields[1] or ":" not in fields[2]:
                continue
            local_hex, local_port = fields[1].split(":", 1)
            peer_hex, peer_port = fields[2].split(":", 1)
            local = _decode_proc_ipv4(local_hex)
            peer = _decode_proc_ipv4(peer_hex)
            if local == selected["localAddress"] and int(local_port, 16) == selected["localPort"]:
                rows.append({"inode": int(fields[9]), "local": local, "peer": peer,
                             "peerPort": int(peer_port, 16),
                             "peerMatches": peer == selected["peerAddress"]
                             and int(peer_port, 16) == selected["peerPort"]})
        return rows
    # IPv6 proc layout is platform/kernel dependent here; retain the raw
    # read privately and report tuple ownership honestly as unknown.
    return []


def _parse_route(text: str, selected: dict) -> dict:
    """Parse one `ip route get` result without treating command success as proof."""

    result = {"observed": False, "destinationMatches": False, "source": None,
              "interface": None, "gateway": None, "sourceMatches": False,
              "tokens": []}
    line = next((line.strip() for line in text.splitlines() if line.strip()), None)
    if line is None:
        return result
    try:
        tokens = shlex.split(line)
    except ValueError:
        tokens = line.split()
    result["tokens"] = tokens
    if not tokens:
        return result
    result["observed"] = True
    result["destinationMatches"] = tokens[0] == selected["peerAddress"]
    for index, token in enumerate(tokens[:-1]):
        if token == "dev":
            result["interface"] = tokens[index + 1]
        elif token in {"src", "from"}:
            result["source"] = tokens[index + 1]
        elif token == "via":
            result["gateway"] = tokens[index + 1]
    result["sourceMatches"] = result["source"] == selected["localAddress"]
    return result


def _overlay_interface(value: object) -> bool:
    return isinstance(value, str) and value.lower().startswith(
        ("tun", "tap", "wg", "warp", "tailscale"))


def _fd_inodes(fd_listing: str) -> dict[int, list[int]]:
    found: dict[int, list[int]] = {}
    for line in fd_listing.splitlines():
        match = re.search(r"(?:^|\s)(\d+)\s+->\s+socket:\[(\d+)\]\s*\Z", line)
        if match:
            found.setdefault(int(match.group(2)), []).append(int(match.group(1)))
    return found


def _read(device: CurrentDevice, item: dict, name: str, *args: str) -> str | None:
    started = utc()
    try:
        stdout, code = device.shell(*args, required=False)
    except Exception as error:
        item.setdefault("errors", []).append({"operation": name, "failureClass": type(error).__name__})
        return None
    item.setdefault("observations", []).append({"name": name, "startedUtc": started,
        "completedUtc": utc(), "exit": code, "stdout": _sanitize_raw(stdout)})
    if code != 0:
        item.setdefault("errors", []).append({"operation": name, "exit": code})
        return None
    return stdout


def _collect_endpoint(endpoint: dict, device: CurrentDevice) -> dict:
    selected = endpoint["selectedTuple"]
    item = {"role": endpoint["role"], "model": endpoint["model"], "transport": endpoint["transport"],
            "actorKind": endpoint["actor"]["kind"], "selectedTuple": selected,
            "observations": [], "errors": [], "socketMark": endpoint["socketEvidence"] or {
                "observed": False, "value": None}, "actualSocketUid": {"observed": False, "value": None},
            "actualProtect": {"observed": False, "value": None}, "startedUtc": utc()}
    model_lines = []
    for property_name in ("ro.product.model", "ro.product.manufacturer", "ro.product.cpu.abilist", "ro.build.version.sdk"):
        property_value = _read(device, item, "modelProperty:" + property_name, "getprop", property_name) or ""
        model_lines.extend(line.strip() for line in property_value.splitlines() if line.strip())
    item["modelObserved"] = bool(model_lines) and model_lines[0].replace(" ", "_") == endpoint["model"]
    item["arm64Observed"] = any("arm64-v8a" in line for line in model_lines)
    if not item["modelObserved"]:
        item["errors"].append({"operation": "model properties", "fact": "selected model was not reread"})
    connectivity = _read(device, item, "connectivity", "dumpsys", "connectivity") or ""
    vpn = _read(device, item, "vpnManagement", "dumpsys", "vpn_management") or ""
    always_on = _read(device, item, "alwaysOnVpn", "settings", "get", "secure", "always_on_vpn_app") or ""
    lockdown = _read(device, item, "vpnLockdown", "settings", "get", "secure", "always_on_vpn_lockdown") or ""
    addresses_text = _read(device, item, "addresses", "ip", "-j", "address", "show") or ""
    v4_routes = _read(device, item, "ipv4Routes", "ip", "-4", "route", "show", "table", "all") or ""
    v6_routes = _read(device, item, "ipv6Routes", "ip", "-6", "route", "show", "table", "all") or ""
    v4_rules = _read(device, item, "ipv4Rules", "ip", "-4", "rule", "show") or ""
    v6_rules = _read(device, item, "ipv6Rules", "ip", "-6", "rule", "show") or ""
    shell_uid = _read(device, item, "shellUid", "id") or ""
    network = _agent_facts(connectivity)
    addresses = _address_facts(addresses_text, network["interfaces"])
    item["network"] = {key: value for key, value in network.items() if key not in {"interfaces", "selectedAgent"}}
    item["network"].update({key: value for key, value in addresses.items() if key != "addresses"})
    item["network"].update({
        "alwaysOnVpnConfigured": always_on.strip() not in ("", "null"),
        "vpnLockdownEnabled": lockdown.strip() == "1",
        "vpnManagementObserved": bool(vpn),
        "policyUidRulesPresent": "uidrange" in (v4_rules + "\n" + v6_rules),
        "policyFwmarkRulesPresent": "fwmark" in (v4_rules + "\n" + v6_rules),
        "shellUidObserved": bool(re.search(r"uid=\d+", shell_uid)),
    })
    item["network"]["activeValidatedNetwork"] = bool(
        item["network"]["activeDefaultObserved"] and item["network"]["defaultAgentObserved"]
        and item["network"]["defaultValidated"]
        and (item["network"]["defaultTransportCellular"] or item["network"]["defaultTransportWifi"])
    )
    item["network"]["activeNotVpnPhysicalNetwork"] = bool(
        item["network"]["activeValidatedNetwork"]
        and item["network"]["defaultNotVpnCapability"]
        and (item["network"]["defaultTransportCellular"] or item["network"]["defaultTransportWifi"])
    )
    item["network"]["defaultInterfaceNamesPrivate"] = network["interfaces"]
    item["routeFacts"] = {"ipv4RouteRows": len(v4_routes.splitlines()), "ipv6RouteRows": len(v6_routes.splitlines()),
                           "ipv4RuleRows": len(v4_rules.splitlines()), "ipv6RuleRows": len(v6_rules.splitlines())}

    if endpoint["role"] == "native":
        directory = endpoint["actor"]["ownedDirectory"]
        pid_text = _read(device, item, "ownedPidBefore", "cat", directory + "/actor.pid") or ""
        if not re.fullmatch(r"[1-9][0-9]*\s*", pid_text):
            item["errors"].append({"operation": "owned PID", "fact": "missing or invalid"})
        else:
            pid = int(pid_text.strip())
            item["ownedPid"] = pid
            status = _read(device, item, "ownedProcessStatus", "sed", "-n", "/^Uid:/p", f"/proc/{pid}/status") or ""
            fd_listing = _read(device, item, "ownedSocketFds", "ls", "-l", f"/proc/{pid}/fd") or ""
            proc_udp = _read(device, item, "ownedUdpTable", "cat", f"/proc/{pid}/net/udp") or ""
            proc_udp6 = _read(device, item, "ownedUdp6Table", "cat", f"/proc/{pid}/net/udp6") or ""
            pid_after = _read(device, item, "ownedPidAfter", "cat", directory + "/actor.pid") or ""
            uid_match = re.fullmatch(r"Uid:\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*", status)
            item["ownedPidUnchangedDuringRead"] = pid_after.strip() == str(pid)
            if uid_match and item["ownedPidUnchangedDuringRead"]:
                values = list(map(int, uid_match.groups()))
                item["ownedProcessUid"] = dict(zip(("real", "effective", "saved", "filesystem"), values))
                item["routeUid"] = values[1]
                item["routeUidProvenance"] = "owned native process effective UID; actual socket UID remains unknown"
            rows = _proc_rows(proc_udp, selected) if ipaddress.ip_address(selected["localAddress"]).version == 4 else []
            fd_map = _fd_inodes(fd_listing)
            owned = [row for row in rows if row["inode"] in fd_map and len(fd_map[row["inode"]]) == 1]
            item["socketOwnership"] = {"tupleRows": len(rows), "uniqueMatchingInodes": len(owned),
                                        "fdNumbers": fd_map.get(owned[0]["inode"], []) if len(owned) == 1 else [],
                                        "inode": owned[0]["inode"] if len(owned) == 1 else None,
                                        "uniqueFdTupleOwned": len(owned) == 1,
                                        "exactPeerRows": sum(row.get("peerMatches") is True for row in rows),
                                        "exactPeerOwned": any(row.get("peerMatches") is True for row in owned),
                                        "ipv6TupleOwnership": False if ipaddress.ip_address(selected["localAddress"]).version == 6 else None}
            evidence = item["socketMark"]
            if evidence.get("observed"):
                item["socketMark"]["correlatedToLiveOwnedPid"] = evidence["pid"] == pid
                item["socketMark"]["correlatedToUniqueTupleInode"] = (
                    len(owned) == 1 and evidence["inode"] == owned[0]["inode"])
                if not (item["socketMark"]["correlatedToLiveOwnedPid"]
                        and item["socketMark"]["correlatedToUniqueTupleInode"]):
                    item["errors"].append({"operation": "socket mark ownership", "fact": "observer does not match live PID/inode"})
    else:
        package = _read(device, item, "exactChromePackageUid", "cmd", "package", "list", "packages", "-U", "com.android.chrome") or ""
        matches = re.findall(r"^package:com\.android\.chrome\s+uid:(\d+)\s*$", package, re.MULTILINE)
        item["chromePackageUidObserved"] = len(matches) == 1
        if len(matches) == 1:
            item["routeUid"] = int(matches[0])
            item["routeUidProvenance"] = "observed Chrome package UID; Chrome network-service socket UID remains unknown"

    if "routeUid" in item:
        family = ipaddress.ip_address(selected["peerAddress"]).version
        route_args = ["ip", f"-{family}", "route", "get", selected["peerAddress"],
                      "from", selected["localAddress"], "uid", str(item["routeUid"])]
        evidence = item["socketMark"]
        if evidence.get("observed") and evidence.get("correlatedToLiveOwnedPid", True) \
                and evidence.get("correlatedToUniqueTupleInode", True):
            route_args += ["mark", hex(evidence["value"])]
            item["routeLookupIncludesObservedMark"] = True
        else:
            item["routeLookupIncludesObservedMark"] = False
        route = _read(device, item, "exactSelectedPeerRoute", *route_args)
        parsed_route = _parse_route(route or "", selected)
        network_interfaces = network["interfaces"]
        interface_matches_default = bool(parsed_route["interface"] and network_interfaces
                                          and parsed_route["interface"] in network_interfaces)
        item["routeLookup"] = {"performed": route is not None, "family": family,
                                "uidObserved": True, "uidIsActualSocketUid": False,
                                "selectedLocalAndPeer": True,
                                "markObserved": item["routeLookupIncludesObservedMark"],
                                "parsed": parsed_route,
                                "interfaceMatchesActiveDefault": interface_matches_default,
                                "overlayInterface": _overlay_interface(parsed_route["interface"]),
                                "actualInterfaceRoute": False}
        # Keep the expression above explicit and precedence-safe for the
        # public predicate; route output itself remains private.
        item["routeLookup"]["actualInterfaceRoute"] = bool(
            parsed_route["observed"] and parsed_route["destinationMatches"]
            and parsed_route["sourceMatches"] and interface_matches_default
            and not item["routeLookup"]["overlayInterface"])
    else:
        item["routeLookup"] = {"performed": False, "uidObserved": False,
                                "uidIsActualSocketUid": False, "selectedLocalAndPeer": True,
                                "markObserved": False, "actualInterfaceRoute": False,
                                "interfaceMatchesActiveDefault": False,
                                "overlayInterface": None}
    item["currentM"] = endpoint["currentM"]
    item["pcStats"] = endpoint["pcStats"]
    pair = endpoint.get("selectedPair")
    pair_nonrelay = bool(pair and pair.get("nonRelay") is True)
    pair_selected = bool(pair and pair.get("local") and pair.get("remote"))
    socket = item.get("socketOwnership", {})
    native_socket_correlated = bool(socket.get("uniqueFdTupleOwned") and (
        socket.get("exactPeerOwned") or socket.get("tupleRows") == 1))
    native_mark_correlated = bool(item.get("socketMark", {}).get("observed")
                                 and item["socketMark"].get("correlatedToLiveOwnedPid")
                                 and item["socketMark"].get("correlatedToUniqueTupleInode"))
    item["qualification"] = {
        "activeNotVpnPhysicalNetwork": bool(item["network"].get("activeNotVpnPhysicalNetwork")),
        "selectedPairObserved": pair_selected,
        "selectedPairNonRelay": pair_nonrelay,
        "selectedRouteToActualInterface": bool(item["routeLookup"].get("actualInterfaceRoute")),
        "nativeSocketTupleOwned": native_socket_correlated if endpoint["role"] == "native" else None,
        "nativeMarkObservedAndCorrelated": native_mark_correlated if endpoint["role"] == "native" else None,
        # Browser package UID/protect and browser kernel socket ownership are
        # deliberately unknown; they are diagnostics, not acceptance gates.
        "browserSocketUidUnknown": endpoint["role"] == "browser",
        "browserProtectUnknown": endpoint["role"] == "browser",
    }
    item["physicalDataQualified"] = bool(
        item["qualification"]["activeNotVpnPhysicalNetwork"]
        and item["qualification"]["selectedPairObserved"]
        and item["qualification"]["selectedPairNonRelay"]
        and item["qualification"]["selectedRouteToActualInterface"]
        and (endpoint["role"] != "native" or item["qualification"]["nativeSocketTupleOwned"])
    )
    # A single endpoint cannot establish different access networks or WAN NAT.
    # Only the pair summary can qualify that relation from both observations.
    item["physicalWanQualified"] = False
    item["limits"] = [
        "ADB and shell reads are control/evidence channels, not data-route proof.",
        "Process/package UIDs are not actual selected socket UIDs.",
        "An omitted SO_MARK observation is unknown, including mark zero.",
        "IPv6 proc tuple ownership is retained as unknown unless the platform emits an exact owner row.",
        "Browser-owned selected-PC stats outrank CDP WebSocket lifecycle, but do not expose socket UID/protect.",
        "Physical data qualification requires an active non-VPN Android network, a selected nonrelay pair, an exact route to the active interface, and native tuple ownership; no Chrome socket UID or protect fact is inferred.",
    ]
    item["commands"] = device.commands
    item["completedUtc"] = utc()
    return item


def _write_private(path: Path, value: dict) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
    path.chmod(0o600)


def _distinct_access_networks(native: dict, browser: dict) -> bool:
    """Qualify only the observed cellular/Wi-Fi scene, never two LAN peers.

    Device-local network IDs, interface names and different private addresses
    cannot prove different access. Require opposite physical default types
    and distinct global peer wire addresses measured on the selected pair.
    Other scenes remain unknown rather than accepting a caller's WAN flag.
    """
    def transport(endpoint):
        network = endpoint.get("network", {})
        cellular = network.get("defaultTransportCellular") is True
        wifi = network.get("defaultTransportWifi") is True
        return "cellular" if cellular and not wifi else "wifi" if wifi and not cellular else None

    if {transport(native), transport(browser)} != {"cellular", "wifi"}:
        return False
    try:
        addresses = [ipaddress.ip_address(value.get("selectedTuple", {}).get("peerAddress"))
                     for value in (native, browser)]
    except ValueError:
        return False
    return all(address.is_global for address in addresses) and addresses[0] != addresses[1]


def _public_summary(report: dict) -> dict:
    endpoints = report.get("endpoints", [])
    by_role = {item.get("role"): item for item in endpoints if isinstance(item, dict)}
    native = by_role.get("native", {})
    browser = by_role.get("browser", {})
    native_qualification = native.get("qualification", {})
    browser_qualification = browser.get("qualification", {})
    physical = bool(
        report.get("commandExit") == 0
        and
        report.get("pairCorrelation") is True
        and native_qualification.get("activeNotVpnPhysicalNetwork") is True
        and browser_qualification.get("activeNotVpnPhysicalNetwork") is True
        and native_qualification.get("selectedPairNonRelay") is True
        and browser_qualification.get("selectedPairNonRelay") is True
        and native_qualification.get("selectedPairObserved") is True
        and browser_qualification.get("selectedPairObserved") is True
        and native_qualification.get("nativeSocketTupleOwned") is True
        and native_qualification.get("nativeMarkObservedAndCorrelated") is True
        and native_qualification.get("selectedRouteToActualInterface") is True
        and browser_qualification.get("selectedRouteToActualInterface") is True)
    distinct_access = _distinct_access_networks(native, browser)
    packet = report.get("packetCorrelation")
    wan = bool(physical and distinct_access
               and isinstance(packet, dict) and packet.get("correlated") is True)
    return {"receipt": report["receipt"], "commandExit": report["commandExit"],
            "endpointCount": len(endpoints),
            "activeValidatedNetworkCount": sum(bool(item.get("network", {}).get("activeValidatedNetwork"))
                                              for item in endpoints),
            "activeNotVpnPhysicalNetworkCount": sum(bool(item.get("network", {}).get("activeNotVpnPhysicalNetwork"))
                                                     for item in endpoints),
            "exactRouteLookupCount": sum(bool(item.get("routeLookup", {}).get("performed"))
                                         for item in endpoints),
            "selectedNonRelayCount": sum(bool(item.get("qualification", {}).get("selectedPairNonRelay"))
                                          for item in endpoints),
            "nativeSocketTupleOwned": native_qualification.get("nativeSocketTupleOwned"),
            "nativeMarkObservedAndCorrelated": native_qualification.get("nativeMarkObservedAndCorrelated"),
            "nativeRouteToActualInterface": native_qualification.get("selectedRouteToActualInterface"),
            "browserRouteToActualInterface": browser_qualification.get("selectedRouteToActualInterface"),
            "distinctAccessNetworksObserved": distinct_access,
            "distinctAccessMethod": "opposite CELLULAR/WIFI defaults and distinct global peer wire addresses",
            "pairCorrelation": report.get("pairCorrelation", False),
            "actualSocketUidObservedCount": sum(bool(item.get("actualSocketUid", {}).get("observed"))
                                                for item in endpoints),
            "actualSocketMarkObservedCount": sum(bool(item.get("socketMark", {}).get("observed"))
                                                 for item in endpoints),
            "physicalDataQualified": physical,
            "physicalWanQualified": wan,
            "routeObservationUnknown": report.get("routeObservationUnknown", False)}


def _development_checks() -> dict:
    """Reject representative guessed/mismatched route witnesses offline."""

    native_tuple = {"localAddress": "10.20.30.4", "localPort": 40000,
                    "peerAddress": "198.51.100.7", "peerPort": 40001}
    browser_tuple = {"localAddress": "192.0.2.8", "localPort": 41000,
                     "peerAddress": "203.0.113.9", "peerPort": 41001}
    def member(selected):
        return {"sameOriginal": True, "alive": True, "memberGate": True,
                "selectedRtc": True, "localTuple": selected}
    valid = {"endpoints": [
        {"role": "native", "model": NATIVE_MODEL, "transport": "usb",
         "actor": {"kind": "native", "ownedDirectory": "/data/local/tmp/miopunch-pair-" + "a" * 32},
         "selectedTuple": native_tuple, "currentM": member(native_tuple),
         "socketEvidence": {"observed": False, "value": None}},
        {"role": "browser", "model": BROWSER_MODEL, "transport": "usb",
         "actor": {"kind": "chrome", "package": "com.android.chrome"},
         "selectedTuple": browser_tuple, "currentM": member(browser_tuple),
         "pcStats": {"source": "browser-owned-selected-pc", "selected": True,
                      "sameOriginal": True, "alive": True, "memberGate": True,
                      "localTuple": browser_tuple, "peerTuple": browser_tuple,
                      "pingCount": 4, "pongCount": 4}},
    ]}
    validate_request(valid)
    rejected = 0
    for mutate in (
        lambda value: value["endpoints"][0].update(model="TB322FC"),
        lambda value: value["endpoints"][1]["pcStats"].update(source="cdp-websocket"),
        lambda value: value["endpoints"][0]["currentM"].update(selectedRtc=False),
        lambda value: value["endpoints"][0].update(transport="network"),
        lambda value: value["endpoints"][0]["socketEvidence"].update(value=0),
        lambda value: value["endpoints"][0]["socketEvidence"].update(
            observed=True, socketEvidence={"operation": "getsockopt(SO_MARK)", "success": True,
                "value": 0, "pid": 1, "localAddress": "203.0.113.1", "localPort": 6553, "inode": 9}),
    ):
        candidate = json.loads(json.dumps(valid))
        mutate(candidate)
        try:
            validate_request(candidate)
        except ValueError:
            rejected += 1
    if rejected != 6:
        raise ValueError("route receipt counterexamples did not reject all six invalid witnesses")
    unknown_mark = validate_request(valid)["endpoints"][0]["socketEvidence"]
    if unknown_mark != {"observed": False, "value": None}:
        raise ValueError("missing mark was promoted to an observed value")
    ipv6 = {"localAddress": "2001:db8::4", "localPort": 40000,
            "peerAddress": "2001:db8::7", "peerPort": 40001}
    if _proc_rows("\n", ipv6):
        raise ValueError("unsupported IPv6 proc ownership was promoted")
    return {"parserCounterexamples": rejected, "missingMarkRemainsUnknown": True,
            "ipv6OwnershipRemainsUnknown": True,
            "adbStarted": False, "physicalWanQualified": False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--development-checks", action="store_true")
    parser.add_argument("--request", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--gap-reason", choices=("rtc-association-local-tuple-unavailable",
                                                   "selected-pc-stats-unavailable",
                                                   "native-selected-pair-unavailable",
                                                   "selected-pair-relay", "selected-pair-mismatch",
                                                   "native-live-m-observer-unavailable"))
    parser.add_argument("--adb", type=Path, default=ADB)
    args = parser.parse_args(argv)
    if args.development_checks:
        # The parser-only path intentionally does not require request/output or
        # start ADB; it is useful before a physical story is released.
        print(json.dumps(_development_checks(), separators=(",", ":")))
        return 0
    if args.output is None:
        parser.error("--output is required for collection")
    if args.gap_reason:
        if args.output.exists():
            parser.error("receipt exists; preserve the earlier observation")
        report = {"scope": "private read-only route collector gap; no device or network operation",
                  "startedUtc": utc(), "sourceSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  "runtimeSha256": hashlib.sha256((ROOT / "scripts/android_pair_runtime_current.py").read_bytes()).hexdigest(),
                  "physicalWanQualified": False, "networkSettingsChanged": False,
                  "routeObservationUnknown": True, "pairCorrelation": False, "gapReason": args.gap_reason,
                  "endpoints": [], "receipt": str(args.output), "completedUtc": utc(), "commandExit": 0}
        _write_private(args.output, report)
        print(json.dumps(_public_summary(report), separators=(",", ":")))
        return 0
    if args.request is None:
        parser.error("--request is required for collection")
    if args.output.exists():
        parser.error("receipt exists; preserve the earlier observation")
    request_bytes = args.request.read_bytes()
    try:
        request = validate_request(json.loads(request_bytes))
    except (ValueError, TypeError, KeyError) as error:
        parser.error(str(error))
    report = {"scope": "private read-only current OP13/Pixel route corroboration; no SDK/public probe",
              "startedUtc": utc(), "sourceSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "runtimeSha256": hashlib.sha256((ROOT / "scripts/android_pair_runtime_current.py").read_bytes()).hexdigest(),
              "requestSha256": hashlib.sha256(request_bytes).hexdigest(), "endpoints": [],
              "pairCorrelation": request.get("pairCorrelation", False),
              "packetCorrelation": request.get("packetCorrelation"),
              "physicalWanQualified": False, "networkSettingsChanged": False, "receipt": str(args.output)}
    for endpoint in request["endpoints"]:
        device = None
        try:
            device = CurrentDevice(endpoint["model"], endpoint["transport"], args.adb, role=endpoint["role"])
            report["endpoints"].append(_collect_endpoint(endpoint, device))
        except Exception as error:
            report["endpoints"].append({"role": endpoint["role"], "model": endpoint["model"],
                "transport": endpoint["transport"], "errors": [{"operation": "read-only collection",
                "failureClass": type(error).__name__}], "physicalWanQualified": False})
        finally:
            if device is not None:
                report["endpoints"][-1].setdefault("commands", device.commands)
    report["completedUtc"] = utc()
    report["commandExit"] = 2 if any(item.get("errors") for item in report["endpoints"]) else 0
    _write_private(args.output, report)
    print(json.dumps(_public_summary(report), separators=(",", ":")))
    return report["commandExit"]


if __name__ == "__main__":
    raise SystemExit(main())
