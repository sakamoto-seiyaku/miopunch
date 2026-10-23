#!/usr/bin/env python3
"""Observe actual onboarding resource contacts with owned, pinned MQTT brokers.

This runtime driver never compiles. MQTT subscriptions and discovery replies
are observed independently of the SDK through real Mosquitto processes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import socket
import subprocess
import time

from check_client_onboarding import Host, OnboardingRunner


IMAGE = "sha256:315e99cbcdc0133a74897e77da0eb7124eec55fc17b617188874e8240738ae39"


def docker(*args, **kwargs):
    return subprocess.run(["rtk", "proxy", "docker", *args], check=True, capture_output=True, **kwargs)


class Broker:
    def __init__(self, runner, index, host_port=None):
        self.name = f"m113-contact-policy-{time.time_ns()}-{index}"
        self.runner = runner
        self.owned = False
        config = runner.output / f"broker-{index}.conf"
        config.write_text("listener 1883 0.0.0.0\nallow_anonymous true\npersistence false\nlog_dest stdout\nlog_type all\n")
        docker("run", "--detach", "--name", self.name, "--publish", f"127.0.0.1:{host_port or ''}:1883",
               "--mount", f"type=bind,src={config},target=/mosquitto/config/mosquitto.conf,readonly", IMAGE)
        self.owned = True
        address = docker("port", self.name, "1883/tcp", text=True).stdout.strip()
        host, port = address.rsplit(":", 1)
        self.url = f"mqtt://{address}/"
        deadline = time.monotonic() + 10
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=1):
                    break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)

    def discovery_contacts(self):
        log = docker("logs", self.name, text=True).stdout
        return sum("New client connected" in line and "as mio26-" in line for line in log.splitlines())

    def active_discovery_contacts(self):
        active = set()
        for line in docker("logs", self.name, text=True).stdout.splitlines():
            connected = re.search(r"New client connected .* as (mio26-\S+) ", line)
            if connected:
                active.add(connected.group(1))
            closed = re.search(r"Client (mio26-\S+) ", line)
            if closed and any(word in line for word in ["disconnected", "closed", "timeout"]):
                active.discard(closed.group(1))
        return len(active)

    def check_reply(self, host):
        scope = hashlib.sha256(b"miopunch/mqtt-discovery/1\0" + bytes.fromhex(host.ready["status"]["network"]) + bytes(8)).hexdigest()
        nonce = bytes.fromhex("a723bd830e0a42d3aaaf461e11130bcc")
        prefix = f"miopunch/bootstrap/1/{scope}"
        command = ["rtk", "proxy", "docker", "exec", self.name, "mosquitto_sub", "-N", "-C", "1", "-W", "4",
                   "-t", prefix + "/reply/" + nonce.hex()]
        subscriber = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            time.sleep(0.2)
            docker("exec", "-i", self.name, "mosquitto_pub", "-t", prefix + "/query", "-s", input=b"MDQ1" + nonce)
            reply, error = subscriber.communicate(timeout=6)
            assert subscriber.returncode == 0, error.decode(errors="replace")
            assert len(reply) == 90 and reply[:20] == b"MDR2" + nonce
            assert reply[20:52].hex() == host.node
        finally:
            if subscriber.poll() is None:
                subscriber.kill()
                subscriber.communicate(timeout=5)

    def close(self):
        if self.owned:
            (self.runner.output / (self.name + ".log")).write_text(docker("logs", self.name, text=True).stdout)
            docker("rm", "--force", self.name)
            self.owned = False
            remaining = subprocess.run(["rtk", "proxy", "docker", "inspect", self.name], capture_output=True)
            assert remaining.returncode != 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--maintenance-probe", type=Path)
    parser.add_argument("--maintenance-probe-sha")
    args = parser.parse_args()
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only, args.focus = True, False, "resources"
    runner = OnboardingRunner(args)
    if args.maintenance_probe:
        assert args.maintenance_probe_sha == hashlib.sha256(args.maintenance_probe.read_bytes()).hexdigest()
    elif args.maintenance_probe_sha:
        parser.error("maintenance probe path is required with its hash")
    assert docker("image", "inspect", "eclipse-mosquitto:2", "--format", "{{.Id}}", text=True).stdout.strip() == IMAGE
    brokers, observations, passed = [], [], False
    try:
        for index in range(2):
            broker = Broker.__new__(Broker)
            brokers.append(broker)
            broker.__init__(runner, index)
        mapped, unrelated = brokers
        facility = dict(logical_id="m113-local-mapping", native_url=mapped.url, wss_url="wss://mqtt.example.invalid/mqtt")
        for mode in ["disabled-wss", "disabled-logical", "signaling-only", "lookup-only", "permitted-raw", "unrelated-permitted-lookup"]:
            before = [broker.discovery_contacts() for broker in brokers]
            uses = ["Lookup"] if mode == "lookup-only" else ["Signaling"] if mode == "signaling-only" else ["Signaling", "Lookup"]
            resources = [dict(url=mapped.url, uses=uses, limitation="owned socket-observed resource fixture")]
            config = dict(defaults=dict(revision=1, resources=resources), replacement=None,
                          disabled=[], disabled_uses=[], lan_discovery="Disabled")
            if mode in ["disabled-wss", "disabled-logical", "unrelated-permitted-lookup"]:
                config["mqtt_facilities"] = [facility]
                alias = facility["wss_url"] if mode == "disabled-wss" else facility["logical_id"]
                config["disabled"] = [[alias, "Signaling"]]
            if mode == "unrelated-permitted-lookup":
                resources.append(dict(url=unrelated.url, uses=["Signaling", "Lookup"], limitation="independently permitted raw lookup"))
            cfg = runner.write_private(mode + "-resources.json", json.dumps(config).encode(), False)
            host = Host(runner, mode, False, "create", runner.path(mode + "-state", False), configuration=cfg)
            host.call("export_recovery_card", path=runner.private_path(mode + "-card.json", False))
            host.call("acknowledge_recovery_saved")
            delta = [broker.discovery_contacts() - count for broker, count in zip(brokers, before)]
            expected = [1, 0] if mode == "permitted-raw" else [0, 1] if mode == "unrelated-permitted-lookup" else [0, 0]
            observation = dict(case=mode, actualDiscoveryContacts=delta, expectedDiscoveryContacts=expected)
            observations.append(observation)
            runner.record(mode, "socket_observation", observation)
            assert delta == expected, observation
            if mode in ["permitted-raw", "unrelated-permitted-lookup"]:
                (mapped if mode == "permitted-raw" else unrelated).check_reply(host)
                observation["actualPublicDiscoveryReplyVerified"] = True
            host.stop()
        if args.maintenance_probe:
            # These aliases retain distinct legacy wire tags, but name one
            # physical broker for local denial and live retirement.
            with socket.socket() as available:
                available.bind(("127.0.0.1", 1883))
            retired = Broker.__new__(Broker)
            brokers.append(retired)
            retired.__init__(runner, 2, host_port=1883)
            resources = [dict(url=url, uses=["Signaling", "Lookup"], limitation="live retirement origin fixture")
                         for url in ["mqtt://127.0.0.1/", retired.url, unrelated.url]]
            config = dict(defaults=dict(revision=1, resources=resources), replacement=None,
                          disabled=[], disabled_uses=[], lan_discovery="Disabled",
                          maintenance=dict(exploration_interval_ms=250, discovery_interval_ms=1000,
                                           contact_timeout_ms=500))
            cfg = runner.write_private("retirement-resources.json", json.dumps(config).encode(), False)
            runner.linux = args.maintenance_probe.resolve()
            runner.record("maintenance-probe", "pin", dict(path=str(runner.linux), sha256=args.maintenance_probe_sha))
            host = Host(runner, "retirement", False, "create", runner.path("retirement-state", False), configuration=cfg)
            assert retired.active_discovery_contacts() == 2
            assert unrelated.active_discovery_contacts() == 1
            host.call("disable_contact_signaling")
            deadline = time.monotonic() + 1
            while retired.active_discovery_contacts() and time.monotonic() < deadline:
                time.sleep(0.05)
            after_retirement = retired.active_discovery_contacts()
            before_queries = [broker.discovery_contacts() for broker in [retired, unrelated]]
            host.call("explore")
            time.sleep(3)
            status = host.call("status")
            queries = [broker.discovery_contacts() - count for broker, count in zip([retired, unrelated], before_queries)]
            observation = dict(case="live-physical-origin-retirement", remainingDisabledSubscriptions=after_retirement,
                               subsequentQueryContacts=queries, discoveryRounds=status["discovery_rounds"],
                               unrelatedSubscriptionRetained=unrelated.active_discovery_contacts() >= 1)
            observations.append(observation)
            runner.record("retirement", "socket_observation", observation)
            assert after_retirement == 0 and queries[0] == 0
            assert queries[1] >= 1 and status["discovery_rounds"] >= 1 and observation["unrelatedSubscriptionRetained"]
            host.stop()
            before_reopen = retired.discovery_contacts()
            reopened = Host(runner, "retirement-reopen", False, "open", host.directory)
            assert reopened.node == host.node and retired.discovery_contacts() == before_reopen
            unrelated.check_reply(reopened)
            reopened.stop()
            observation["savedDenialRetainedOnReopen"] = True
            observation["unrelatedReplyVerifiedAfterReopen"] = True
        passed = True
    finally:
        cleanup = runner.cleanup()
        for broker in brokers:
            try:
                broker.close()
                cleanup.append(dict(broker=broker.name, removed=True))
            except Exception as error:
                cleanup.append(dict(broker=getattr(broker, "name", "starting"), error=type(error).__name__))
        passed = passed and all("error" not in entry for entry in cleanup)
        result = dict(passed=passed, brokerImage=IMAGE, observations=observations, cleanup=cleanup,
                      privateExportsRemoved=not any((runner.output / name).exists() for name in runner.private))
        (runner.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    assert passed


if __name__ == "__main__":
    main()
