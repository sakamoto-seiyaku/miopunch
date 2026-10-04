#!/usr/bin/env python3
"""Five run-owned containers and four internal bridges; never host networking."""
import ipaddress
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

from check_browser_rtc import ROOT

IMAGE = "sha256:c9dbb88a37f58ae5105720e60bc5cc6eb40d8ad37106e57ce621d345b5ec62a2"
REMOVAL_RESERVE_SECONDS = 9


def topology(subnets):
    networks = {}
    for role, subnet in zip(("lan-a", "lan-b", "wan-a", "wan-b"), subnets):
        network = ipaddress.ip_network(subnet)
        networks[role] = {"subnet": subnet, "gateway": str(network.network_address + 1)}
    address = lambda role, offset: str(ipaddress.ip_network(networks[role]["subnet"]).network_address + offset)
    return {"networks": networks, "roles": {
        "browser": {"links": {"lan-a": address("lan-a", 2)}, "default": address("lan-a", 254)},
        "native": {"links": {"lan-b": address("lan-b", 2)}, "default": address("lan-b", 254)},
        "nat-a": {"links": {"lan-a": address("lan-a", 254), "wan-a": address("wan-a", 2)}, "default": address("wan-a", 254)},
        "nat-b": {"links": {"lan-b": address("lan-b", 254), "wan-b": address("wan-b", 2)}, "default": address("wan-b", 254)},
        "router": {"links": {"wan-a": address("wan-a", 254), "wan-b": address("wan-b", 254)}, "default": None},
    }, "service": str(ipaddress.ip_network(networks["wan-a"]["subnet"]).network_address - 11 * 256 + 10),
        "model": "one NAT per endpoint; explicit permissive SNAT/UDP-DNAT/filter rules, no measured NAT-class or WAN claim"}


class OwnedLab:
    def __init__(self, output, fixture, target, *, mode, total=120):
        self.output, self.fixture, self.target = output, fixture, target
        self.mode, self.total = mode, total
        self.started = time.monotonic()
        self.owner = "mio-rtc-s11-" + str(time.time_ns())
        self.containers, self.networks, self.commands = {}, {}, []
        self.cleanup_results = []
        self.sequence = 0
        self.topology = None
        self.stop_slots = 1

    def remaining(self, *, cleanup=False):
        left = self.total - (0 if cleanup else 15) - (time.monotonic() - self.started)
        if left <= 0:
            raise TimeoutError("owned STEP11 clock exhausted; cleanup reserve is15s")
        return left

    def stop_budget(self):
        available = self.total - (time.monotonic()-self.started) - REMOVAL_RESERVE_SECONDS
        return max(0, min(2, available/max(1, self.stop_slots)))

    def command(self, name, argv, *, budget=5, capture=False, cleanup=False, terminal=False, expected=(0,)):
        self.sequence += 1
        path = self.output / f"{self.sequence:03d}-{name}.log"
        started = time.monotonic()
        entry = {"name": name, "argv": argv, "log": path.name}
        self.commands.append(entry)
        try:
            with path.open("w") as log:
                # Terminal removal/absence cannot be suppressed by an already
                # failed outer clock. Its small fixed budgets are recorded;
                # any total overrun still makes the case fail qualification.
                limit = budget if terminal else min(budget, self.remaining(cleanup=cleanup))
                entry.update(timeoutSeconds=limit, terminalCleanup=terminal,
                    remainingTotalSecondsAtLaunch=self.total-(time.monotonic()-self.started))
                completed = subprocess.Popen(["rtk", "proxy", *argv], cwd=ROOT, text=True,
                    stdout=subprocess.PIPE if capture else log, stderr=log, start_new_session=True)
                entry["processGroup"] = completed.pid
                try:
                    stdout, _ = completed.communicate(timeout=limit)
                except BaseException:
                    for signum in (signal.SIGTERM, signal.SIGKILL):
                        try:
                            os.killpg(completed.pid, signum)
                        except ProcessLookupError:
                            pass
                        try:
                            completed.communicate(timeout=0.1)
                        except subprocess.TimeoutExpired:
                            continue
                    # Also kill descendants that survived their reaped proxy.
                    try:
                        os.killpg(completed.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    completed.wait(timeout=0.1)
                    entry.update(exit=completed.returncode, groupTerminationAttempted=True)
                    raise
                if capture:
                    log.write(stdout or "")
            entry.update(exit=completed.returncode, seconds=time.monotonic()-started)
            if completed.returncode not in expected:
                raise RuntimeError(f"{name} exited{completed.returncode}; see {path}")
            return stdout if capture else None
        except BaseException as error:
            entry.update(error=str(error), seconds=time.monotonic()-started)
            raise
        finally:
            (self.output / "topology-commands.json").write_text(json.dumps(self.commands, indent=2)+"\n")

    def exec(self, role, argv, *, name=None, capture=False, user=None, budget=5, cleanup=False):
        command = ["docker", "exec"]
        if user is not None:
            command += ["--user", user]
        command += [self.containers[role], "rtk", "proxy", *argv]
        return self.command(name or role+"-"+Path(argv[0]).name, command,
            budget=budget, capture=capture, cleanup=cleanup)

    def start(self):
        identity = self.command("image", ["docker", "image", "inspect", IMAGE, "--format", "{{.Id}}"], capture=True).strip()
        if identity != IMAGE:
            raise RuntimeError("cached image identity differs")
        ids = self.command("network-ids", ["docker", "network", "ls", "--format", "{{.ID}}"], capture=True).split()
        existing = []
        if ids:
            lines = self.command("network-subnets", ["docker", "network", "inspect", *ids,
                "--format", "{{json .IPAM.Config}}"], capture=True)
            records = [json.loads(line) for line in lines.splitlines() if line.strip()]
            existing = [ipaddress.ip_network(entry["Subnet"]) for record in records for entry in (record or []) if entry.get("Subnet")]
        for proposal in [
            ("10.203.1.0/24", "10.203.2.0/24", "198.18.11.0/24", "198.18.12.0/24"),
            ("10.204.1.0/24", "10.204.2.0/24", "198.19.11.0/24", "198.19.12.0/24"),
        ]:
            if not any(ipaddress.ip_network(candidate).overlaps(current)
                       for candidate in proposal for current in existing if current.version == 4):
                self.topology = topology(proposal)
                break
        if self.topology is None:
            raise RuntimeError("both owned topology ranges overlap existing Docker networks")
        self.topology.update(mode=self.mode, owner=self.owner)
        configuration = self.fixture / "topology.json"
        configuration.write_text(json.dumps(self.topology, indent=2)+"\n")
        configuration.chmod(0o644)
        (self.output / "topology.json").write_text(json.dumps(self.topology, indent=2)+"\n")
        for role, settings in self.topology["networks"].items():
            name = self.owner+"-"+role
            self.networks[role] = name
            self.command("create-"+role, ["docker", "network", "create", "--internal", "--subnet", settings["subnet"],
                "--gateway", settings["gateway"], "--label", "miopunch.run="+self.owner, name])
        node = Path(shutil.which("node")).resolve().parent.parent
        rtk = Path(shutil.which("rtk")).resolve()
        chrome = next((Path.home()/".cache/ms-playwright").glob("chromium-1243/chrome-linux64"))
        for role, settings in self.topology["roles"].items():
            name = self.owner+"-"+role
            self.containers[role] = name
            first_network, first_address = next(iter(settings["links"].items()))
            # Docker forbids adding a bridge to an existing NetworkMode=none
            # container. Start only on the first NEW internal bridge instead;
            # no application starts until all own routes/filters are installed.
            argv = ["docker", "run", "--detach", "--name", name, "--network", self.networks[first_network], "--ip", first_address, "--label", "miopunch.run="+self.owner,
                    "--cap-drop", "ALL", "--cap-add", "NET_ADMIN", "--workdir", "/repo",
                    "--add-host", "mio-mqtt.test:"+self.topology["service"], "--add-host", "mio-relay.test:"+self.topology["service"],
                    "--env", "PATH=/opt/node/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"]
            if role.startswith("nat-") or role == "router":
                argv += ["--cap-add", "NET_RAW", "--sysctl", "net.ipv4.ip_forward=1"]
            mounts = [(ROOT, "/repo", True), (rtk, "/usr/local/bin/rtk", True),
                      (configuration, "/topology.json", True), (self.output, "/evidence", False)]
            if role in ("browser", "native", "router"):
                mounts += [(self.fixture, "/fixture", False), (self.target, "/build", True)]
            if role in ("browser", "router"):
                mounts += [(node, "/opt/node", True)]
            if role == "browser":
                for directory in ("chrome-config", "chrome-cache"):
                    (self.fixture/directory).mkdir(mode=0o700, exist_ok=True)
                account = self.fixture/"browser-passwd"
                account.write_text(f"root:x:0:0:root:/root:/bin/sh\nmiofixture:x:{os.getuid()}:{os.getgid()}:owned fixture:/root:/usr/sbin/nologin\n")
                account.chmod(0o644)
                mounts += [(chrome, "/opt/chromium", True), (Path("/usr/lib/x86_64-linux-gnu"), "/opt/hostlib", True),
                           (account, "/etc/passwd", True),
                           (self.fixture/"chrome-config", "/root/.config", False), (self.fixture/"chrome-cache", "/root/.cache", False),
                           (self.fixture/"nssdb", "/root/.pki/nssdb", False)]
                argv += ["--env", "PLAYWRIGHT_CHROMIUM_EXECUTABLE=/opt/chromium/chrome", "--env", "MIO_PRIVATE_NSS_NAMESPACE=1"]
            for source, destination, readonly in mounts:
                argv += ["--mount", f"type=bind,src={source},target={destination}"+(",readonly" if readonly else "")]
            argv += ["--entrypoint", "/bin/sleep", IMAGE, str(self.total)]
            self.command("start-"+role, argv)
            for network, address in settings["links"].items():
                if network == first_network:
                    continue
                self.command("attach-"+role+"-"+network, ["docker", "network", "connect", "--ip", address, self.networks[network], name])
            self.exec(role, ["python3", "/repo/scripts/rtc_cross_network_configure.py", role], name="configure-"+role, budget=8)
        namespaces = {}
        for role in self.containers:
            namespaces[role] = json.loads(self.exec(role, ["python3", "/repo/scripts/rtc_cross_network_configure.py", role, "snapshot"],
                name="snapshot-"+role, capture=True))
        if len({entry["netNamespace"] for entry in namespaces.values()}) != 5:
            raise RuntimeError("five containers do not own five distinct network namespaces")
        (self.output/"namespace-before.json").write_text(json.dumps(namespaces, indent=2)+"\n")
        self.output.chmod(0o777)  # Public capture files only; private fixture remains0700.
        for role in ("nat-a", "nat-b", "router"):
            self.exec(role, ["python3", "/repo/scripts/rtc_cross_network_configure.py", role, "capture-start"], name="capture-"+role)

    def snapshot(self, name, *, cleanup=False):
        snapshots = {}
        for role in ("nat-a", "nat-b", "router"):
            snapshots[role] = json.loads(self.exec(role, ["python3", "/repo/scripts/rtc_cross_network_configure.py", role, "snapshot"],
                name=name+"-"+role, capture=True, cleanup=cleanup))
        (self.output/(name+".json")).write_text(json.dumps(snapshots, indent=2)+"\n")
        return snapshots

    def close(self):
        def attempt(name, argv, *, absent=False, terminal=False, budget=0.5):
            try:
                value = self.command(name, argv, budget=budget, capture=absent, cleanup=True, terminal=terminal)
                entry = {"name": name, "passed": not absent or not value.strip()}
            except BaseException as error:
                entry = {"name": name, "passed": False, "error": str(error)}
            self.cleanup_results.append(entry)
        for role in ("nat-a", "nat-b", "router"):
            if role in self.containers:
                if self.total-(time.monotonic()-self.started) <= REMOVAL_RESERVE_SECONDS:
                    self.cleanup_results.append({"name": "stop-capture-"+role, "passed": False,
                        "skipped": "preserve terminal removal/absence tail; capture may be incomplete"})
                else:
                    attempt("stop-capture-"+role, ["docker", "exec", self.containers[role], "rtk", "proxy", "python3",
                            "/repo/scripts/rtc_cross_network_configure.py", role, "capture-stop"])
        if self.containers:
            attempt("remove-containers", ["docker", "rm", "--force", *self.containers.values()], terminal=True, budget=2)
        if self.networks:
            attempt("remove-networks", ["docker", "network", "rm", *self.networks.values()], terminal=True, budget=2)
        attempt("containers-absent", ["docker", "ps", "-a", "--filter", "label=miopunch.run="+self.owner, "--format", "{{.ID}}"], absent=True, terminal=True)
        attempt("networks-absent", ["docker", "network", "ls", "--filter", "label=miopunch.run="+self.owner, "--format", "{{.ID}}"], absent=True, terminal=True)
        self.output.chmod(0o755)
        return all(entry["passed"] for entry in self.cleanup_results)
