"""Read-only live Host/Pixel route evidence; upstream Windows route stays unknown."""

import hashlib
import json
from pathlib import Path
import re
import subprocess

import android_pair_route_receipt_current as android
from android_pair_packet_witness import correlate_selected_tuples


def route_matches(rows, selected, defaults):
    """A successful route command alone does not establish the selected route."""
    if not isinstance(rows, list) or len(rows) != 1:
        return False
    route = rows[0]
    return bool(isinstance(route, dict)
                and route.get("dst") == selected["peerAddress"]
                and route.get("prefsrc", route.get("src", route.get("from"))) == selected["localAddress"]
                and route.get("dev") in defaults
                and not android._overlay_interface(route.get("dev"))
                and route.get("type", "unicast") == "unicast")


def collect_host(endpoint, actor):
    item = {"scope": "Host/Linux kernel", "observations": [],
            "selectedRouteToDefaultInterface": False,
            "selectedSocketOwnedAndMarkCorrelated": False,
            "upstreamWindowsRouteObserved": False, "physicalWanQualified": False}
    try:
        return _collect_host(endpoint, actor, item)
    except Exception as error:
        item["observerFailure"] = {"failureClass": type(error).__name__, "message": str(error)[:256]}
        return item


def _collect_host(endpoint, actor, item):
    selected = android._tuple(endpoint["selectedTuple"], allow_ipv6=False)
    android._current_member(endpoint["currentM"], selected)
    mark = android._socket_evidence(endpoint.get("socketEvidence"), selected, "native")
    directory = Path(actor.directory)
    if not re.fullmatch(r"/tmp/miopunch-host-pair-[0-9a-f]{32}", str(directory)):
        raise ValueError("Host observer requires the exact owned actor directory")
    item.update(selectedTuple=selected, socketMark=mark, currentM=endpoint["currentM"])

    def read(name, *argv):
        result = subprocess.run(["rtk", "proxy", *argv], capture_output=True, text=True, timeout=5)
        item["observations"].append({"name": name, "command": list(argv),
                                     "exit": result.returncode, "stdout": result.stdout,
                                     "stderr": result.stderr})
        if result.returncode:
            raise RuntimeError("Host route read failed: " + name)
        return result.stdout

    pid_text = read("ownedPid", "cat", str(directory / "actor.pid")).strip()
    if not re.fullmatch(r"[1-9][0-9]*", pid_text):
        raise ValueError("Host owned PID is invalid")
    pid = int(pid_text)
    process = Path("/proc") / pid_text
    before = read("processStartBefore", "cat", str(process / "stat"))
    cmdline = (process / "cmdline").read_bytes().split(b"\0")
    if not cmdline or cmdline[0].decode() != actor.binary:
        raise ValueError("Host PID is not the exact owned SDK executable")
    executable = Path(read("processExecutable", "readlink", str(process / "exe")).strip())
    if executable != Path(actor.binary) or hashlib.sha256(executable.read_bytes()).hexdigest() != actor.artifacts[Path(actor.binary).name]:
        raise ValueError("Host executable differs from the pinned SDK")
    fds = android._fd_inodes(read("ownedFds", "ls", "-l", str(process / "fd")))
    udp = read("ownedUdp", "cat", str(process / "net/udp"))
    rows = android._proc_rows(udp, selected)
    owned = [row for row in rows if len(fds.get(row["inode"], [])) == 1]
    socket_owned = bool(len(owned) == 1 and mark and mark["observed"]
                        and mark["pid"] == pid and mark["inode"] == owned[0]["inode"])
    item["selectedSocketOwnedAndMarkCorrelated"] = socket_owned
    if not socket_owned:
        raise ValueError("Host selected socket does not match the live PID/inode/mark")
    # /proc reports the socket UID on this exact inode, rather than a shell UID.
    uid_rows = [line.split() for line in udp.splitlines()[1:]
                if len(line.split()) >= 10 and line.split()[9] == str(mark["inode"])]
    if len(uid_rows) != 1 or not uid_rows[0][7].isdigit():
        raise ValueError("Host selected socket UID is ambiguous")
    uid = int(uid_rows[0][7])
    item["actualSocketUid"] = {"observed": True, "value": uid, "source": "owned selected UDP inode"}
    defaults = json.loads(read("defaultRoutes", "ip", "-j", "-4", "route", "show", "default"))
    interfaces = {v["dev"] for v in defaults if isinstance(v, dict) and isinstance(v.get("dev"), str)}
    addresses = json.loads(read("addresses", "ip", "-j", "-4", "address", "show"))
    route = json.loads(read("selectedRoute", "ip", "-j", "-4", "route", "get",
                           selected["peerAddress"], "from", selected["localAddress"],
                           "mark", str(mark["value"]), "uid", str(uid)))
    local_on_default = any(v.get("ifname") in interfaces and any(
        a.get("local") == selected["localAddress"] for a in v.get("addr_info", [])) for v in addresses)
    after = read("processStartAfter", "cat", str(process / "stat"))
    # Start time is after the final ')' in stat, whose comm field may contain spaces.
    start = lambda value: value.rsplit(")", 1)[1].split()[19]
    unchanged = start(before) == start(after) and read("ownedPidAfter", "cat", str(directory / "actor.pid")).strip() == pid_text
    item["ownedProcessUnchangedDuringRead"] = unchanged
    item["selectedRouteToDefaultInterface"] = bool(unchanged and local_on_default and route_matches(route, selected, interfaces))
    return item


def collect(request, actor, browser_device):
    if not isinstance(request, dict) or "packetWitness" not in request:
        raise ValueError("Host/Pixel observation requires the two live capture snapshots")
    endpoints = request["endpoints"]
    if len(endpoints) != 2 or [v["role"] for v in endpoints] != ["native", "browser"]:
        raise ValueError("Host/Pixel observation requires the exact current roles")
    native, browser = endpoints
    if native["model"] != "LinuxHost" or native["transport"] != "local" or native["actor"] != {
            "kind": "native", "ownedDirectory": actor.directory}:
        raise ValueError("Host route request does not match the owned local actor")
    if browser["model"] != android.BROWSER_MODEL or browser["transport"] != "usb" or browser["actor"] != {
            "kind": "chrome", "package": "com.android.chrome"}:
        raise ValueError("Host/Pixel request does not match the current browser")
    for endpoint in endpoints:
        selected = android._tuple(endpoint["selectedTuple"])
        android._current_member(endpoint["currentM"], selected)
        pair = android._selected_pair(endpoint["selectedPair"])
        if not pair["nonRelay"] or {"localAddress": pair["local"]["baseAddress"],
                "localPort": pair["local"]["basePort"], "peerAddress": pair["remote"]["address"],
                "peerPort": pair["remote"]["port"]} != selected:
            raise ValueError("Host/Pixel selected pair does not match its live tuple")
    browser = dict(browser, socketEvidence=None,
                   pcStats=android._pc_stats(browser["pcStats"], browser["selectedTuple"]))
    packet = correlate_selected_tuples(request["packetWitness"], native["selectedTuple"], browser["selectedTuple"])
    host = collect_host(native, actor)
    pixel = android._collect_endpoint(browser, browser_device)
    summary = {"hostKernelRouteQualified": host["selectedRouteToDefaultInterface"],
               "hostSelectedSocketOwned": host["selectedSocketOwnedAndMarkCorrelated"],
               "pixelActiveNonVpnNetwork": pixel["qualification"]["activeNotVpnPhysicalNetwork"],
               "pixelRouteQualified": pixel["qualification"]["selectedRouteToActualInterface"],
               "twoEndSelectedTuplePacketsCorrelated": packet["correlated"],
               "physicalDataQualified": False, "physicalWanQualified": False,
               "hostUpstreamRouteUnknown": True}
    return {"scope": "Live Host/Linux kernel and Pixel Android; Windows upstream unobserved",
            "host": host, "pixel": pixel, "packetCorrelation": packet, "summary": summary}
