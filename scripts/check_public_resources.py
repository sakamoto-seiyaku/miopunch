#!/usr/bin/env python3
"""Real Linux/Android public services. No local broker/STUN or injected success."""
import argparse
import copy
import json
import pathlib
import shlex
import socket
import subprocess
import time

from check_android_management import Lab as PhysicalLab
from check_desktop_management import Process
from public_relay_proxy import PublicRelayProxy

MQTT_ID = 0x6D696F6D717474


class PublicLab(PhysicalLab):
    def __init__(self, args):
        super().__init__(args)
        self.adb_run("push", args.android_lib, self.remote + "/libnode.so")
        self.adb_run("push", args.android_shell, self.remote + "/shell.jar")
        self.observations = {}
        self.configs = {}
        self.state = {}
        self.receipt = {"started_unix_ms": int(time.time() * 1000), "passed": False,
                        "selected_resources": {"mqtt": args.broker, "stun_tcp": args.stun_tcp, "stun_udp": args.stun_udp, "relay": args.relay},
                        "tls": {"mqtt": "unencrypted public carrier; private exchange uses inner identity-authenticated QUIC/TLS", "relay": "verified HTTPS with embedded WebPKI roots; no verification bypass"}}
        self.save()
        from urllib.parse import urlparse
        self.relay_proxy = PublicRelayProxy([urlparse(args.relay).hostname,
            *[f"{region}-1.relay.n0.iroh.link." for region in ["aps1", "euc1", "use1", "usw1"]]])
        topology = {"linux": subprocess.check_output(["ip", "-j", "address"], text=True),
                    "android": self.shell("ip -4 addr; ip -4 rule; cmd wifi status"),
                    "same_lan": args.android_ip.rsplit(".", 1)[0] == args.linux_ip.rsplit(".", 1)[0],
                    "independent_network": "Not supplied; no active cellular interface. Public relay is mandatory; LAN is excluded by runtime policy."}
        (self.output / "topology.json").write_text(json.dumps(topology, indent=2))
        dns = {}
        for service in [args.broker, args.stun_tcp, args.stun_udp, args.relay]:
            from urllib.parse import urlparse
            host = urlparse(service).hostname
            try:
                dns[service] = sorted({record[4][0] for record in socket.getaddrinfo(host, None)})
            except OSError as error:
                dns[service] = {"error": str(error)}
        (self.output / "linux-dns.json").write_text(json.dumps(dns, indent=2))

    def save(self):
        (self.output / "results.json").write_text(json.dumps(self.receipt, indent=2))

    def command(self, platform, *args):
        if platform == "android" and args[:2] == ("network", "session"):
            command = ["app_process", "/system/bin", "MioPublicShell", self.remote + "/libnode.so", args[2], args[3]]
            return self.adb + ["shell", "CLASSPATH=" + shlex.quote(self.remote + "/shell.jar") + " " + shlex.join(command)]
        if platform == "linux" and args[:2] == ("network", "session"):
            return ["env", "MIOPUNCH_RELAY_PROXY=" + self.relay_proxy.url, self.args.linux_bin, *args]
        return super().command(platform, *args)

    def session(self, platform, name, mode="session"):
        assert mode == "session"
        command = self.command(platform, "network", mode, self.path(platform, name), "0.0.0.0")
        if platform == "android":
            command[-1] = "echo MIOPID $$; exec " + command[-1].replace("CLASSPATH=" + shlex.quote(self.remote + "/shell.jar") + " ", "env CLASSPATH=" + shlex.quote(self.remote + "/shell.jar") + " ", 1)
        process = Process(command, self.log)
        process.platform = platform
        self.processes.append(process)
        if platform == "android":
            process.remote_pid = int(process.wait(lambda e: e.get("raw", "").startswith("MIOPID "))["raw"].split()[1])
        ready = process.wait(lambda e: e.get("event") == "ready")
        return process, ready

    def result(self, process, operation, **args):
        result = process.command(operation, **args)
        assert result["ok"], (operation, result)
        return result["result"]

    @staticmethod
    def relay_address(ready):
        address = copy.deepcopy(ready["address"])
        address["addrs"] = [a for a in address["addrs"] if "Relay" in a]
        assert address["addrs"], "no public relay address"
        return address

    def mqtt_address(self, process, ready):
        status = self.result(process, "address")
        mqtt = [a for a in status["custom"] if a["id"] == MQTT_ID]
        assert mqtt and all(row[1] for row in status["mqtt"]), "public MQTT SUBACK must advertise the route"
        return {"id": ready["node"], "addrs": [{"Custom": a} for a in mqtt]}

    def public_transport(self, result):
        transport = result.get("transport", result)
        assert transport["paths"] and any(path["selected"] for path in transport["paths"]), transport
        assert all("Relay" in path["remote"] and path["remote"]["Relay"].startswith("https://") for path in transport["paths"]), "LAN, direct TCP and signaling must not qualify public data"
        return transport

    def restart(self, platform, name, process, config):
        self.result(process, "save_resources", config=config)
        process.close()
        resumed, ready = self.session(platform, name)
        assert self.result(resumed, "resources")["config"] == config
        return resumed, ready

    def pair(self):
        self.run("linux", "network", "create", self.path("linux", "owner"))
        admin, ar = self.session("linux", "owner")
        default = self.result(admin, "resources")["config"]
        assert default["replacement"] is None and default["defaults"]["resources"]
        default["udp_direct_disabled"] = True
        admin, ar = self.restart("linux", "owner", admin, default)
        self.result(admin, "issue_precode", path=self.path("linux", "code.json"), seconds=3600, max_uses=2)
        self.transfer("linux", "android", "code.json", self.relay_address(ar))
        self.run("android", "network", "join", self.path("android", "member"), self.path("android", "code.json"))
        phone, pr = self.session("android", "member")
        default_android = self.result(phone, "resources")["config"]
        assert default_android["defaults"] == default["defaults"] and default_android["replacement"] is None
        default_android["udp_direct_disabled"] = True
        phone, pr = self.restart("android", "member", phone, default_android)
        assert self.result(phone, "request", path=self.path("android", "code.json"))["stage"] == "Complete"
        assert self.result(admin, "precode_status", path=self.path("linux", "code.json"))["uses"] == 1
        ping = self.result(phone, "ping", address=self.relay_address(ar))
        self.public_transport(ping)
        self.receipt["default_public_data"] = {"android": self.result(phone, "status"), "linux": self.result(admin, "status"), "business": ping}
        self.save()
        return admin, ar, phone, pr

    def profile(self, config, platform, udp):
        config = copy.deepcopy(config)
        config["replacement"] = [
            {"url": self.args.relay, "uses": ["DataRelay"], "limitation": "number0 public encrypted relay; best effort; no availability promise"},
            {"url": self.args.broker, "uses": ["Signaling"], "limitation": "public test broker; no sensitive outer plaintext; inner TLS/QUIC"},
            {"url": self.args.stun_tcp, "uses": ["Observation"], "limitation": "RFC8489 Binding on public TURN listener; no allocation; TCP-specific mapping"},
            {"url": self.args.stun_udp, "uses": ["Observation"], "limitation": "UDP STUN; report failure independently from QAD/TCP"},
        ]
        config["udp_direct_disabled"] = not udp
        config["tcp_listen"] = "0.0.0.0:" + ("41653" if platform == "android" else "41654")
        config["disabled_tcp_addresses"] = [f"{ip}:{port}" for ip in ["0.0.0.0", self.args.linux_ip, self.args.android_ip] for port in [41653, 41654]]
        return config

    def quiet_mqtt(self, admin, phone):
        previous, stable = None, 0
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            current = {"linux": self.result(admin, "address")["mqtt"], "android": self.result(phone, "address")["mqtt"]}
            assert all(row[1] for rows in current.values() for row in rows), "subscription must remain ready"
            stable = stable + 1 if current == previous else 0
            if stable >= 3:
                return current
            previous = current
            time.sleep(0.2)
        raise AssertionError("MQTT QUIC packet counts did not settle")

    def first_and_priority(self):
        admin, ar, phone, pr = self.pair()
        for platform, name, process in [("linux", "owner", admin), ("android", "member", phone)]:
            config = self.profile(self.result(process, "resources")["config"], platform, True)
            process, ready = self.restart(platform, name, process, config)
            observations = self.result(process, "observe")
            assert any(row["protocol"] == "STUN/TCP/RFC8489" and row.get("fresh") for row in observations["observations"]), observations
            self.observations[platform] = observations
            self.receipt["observations"] = self.observations
            self.save()
            config["udp_direct_disabled"] = True
            process, ready = self.restart(platform, name, process, config)
            self.configs[platform] = config
            self.state[platform] = self.result(process, "status")
            final_observation = self.result(process, "observe")
            assert any(row["protocol"] == "STUN/TCP/RFC8489" and row.get("fresh") for row in final_observation["observations"]), final_observation
            self.observations[platform + "_relay_only"] = final_observation
            self.save()
            if platform == "linux": admin, ar = process, ready
            else: phone, pr = process, ready
        self.receipt["observations"] = self.observations
        mqtt_target = self.mqtt_address(admin, ar)
        opened = self.result(phone, "open_business", address=mqtt_target, hints_only=True)
        self.public_transport(opened)
        # A new signed record must actually be delivered and saved via LogSync.
        before = self.result(phone, "status")["records"]
        target = self.run("linux", "identity", self.path("linux", "offline-author-target")).strip()
        published = self.result(admin, "manage", action="add_member", target=target)
        synchronized = self.result(phone, "sync", address=mqtt_target, hints_only=True)
        assert synchronized["complete"]
        self.public_transport(synchronized)
        assert self.result(phone, "status")["records"] == before + 1
        counts = self.quiet_mqtt(admin, phone)
        assert all(row[2] > 0 and row[3] > 0 for rows in counts.values() for row in rows)
        business = self.result(phone, "business")
        self.public_transport(business)
        assert business["transport"]["connection"] == opened["connection"]
        assert self.quiet_mqtt(admin, phone) == counts, "business must not use the public signaling carrier"
        self.receipt["first"] = {"passed": True, "opened": opened, "business": business, "logsync": synchronized,
                                 "saved_record": published, "records_before": before, "records_after": before + 1,
                                 "mqtt_after_control": counts}
        self.save()
        # Exactly one fault: blackhole this run's Linux public-relay tunnel.
        # MQTT/STUN and unrelated endpoints never use this local TCP fixture.
        before_cut = self.relay_proxy.snapshot()
        assert any(row["forwarded_to_public"] > 0 and row["forwarded_from_public"] > 0 for row in before_cut)
        self.relay_proxy.cut.set()
        started = time.monotonic()
        failed = phone.command("business")
        assert not failed["ok"] and time.monotonic() - started < 6, failed
        assert self.quiet_mqtt(admin, phone) == counts, "data loss must not fall into MQTT"
        deadline = time.monotonic() + 18
        while True:
            dead = self.result(phone, "business_status")
            if dead["close_reason"]:
                break
            assert time.monotonic() < deadline, ("old connection must end within idle bound", dead)
            time.sleep(0.5)
        cut = self.relay_proxy.snapshot()
        assert any(row["dropped"] > 0 for row in cut), "the fault must have dropped actual public-relay bytes"
        control_started = time.monotonic()
        control_only = phone.command("open_business", address=self.mqtt_address(admin, ar), hints_only=True)
        assert not control_only["ok"] and time.monotonic() - control_started < 28, control_only
        assert self.result(phone, "business_status")["transport"] is None, "control-only bootstrap cannot release a business handle"
        control_counts = self.quiet_mqtt(admin, phone)
        assert any(row[2] > old[2] for row, old in zip(control_counts["android"], counts["android"])), "the failed attempt must actually use public MQTT control"
        self.relay_proxy.cut.clear()
        phone.close()
        phone, resumed = self.session("android", "member")
        assert resumed["node"] == pr["node"]
        assert self.result(phone, "resources")["config"] == self.configs["android"]
        saved = self.result(phone, "status")
        assert saved["stage"] == "Complete" and saved["records"] == before + 1
        assert self.result(admin, "precode_status", path=self.path("linux", "code.json"))["uses"] == 1
        recovered = self.result(phone, "open_business", address=self.mqtt_address(admin, ar), hints_only=True)
        self.public_transport(recovered)
        assert recovered["connection"] != opened["connection"]
        assert self.result(phone, "business")["reply"] == "PONG"
        self.receipt["priority"] = {"passed": True, "single_fault": "blackhole owned Linux CONNECT tunnel carrying real public relay TLS traffic",
                                    "cut": cut, "failure": failed, "closed": dead, "recovered": recovered,
                                    "control_only_failure": control_only, "control_only_mqtt": control_counts,
                                    "same_node": resumed["node"], "precode_uses": 1, "saved_record_count": saved["records"], "saved_resources_retained": True}
        self.save()
        # Disables survive reopen and cannot be re-enabled as fallback.
        disabled = copy.deepcopy(self.configs["android"])
        disabled["disabled_uses"] = ["Observation", "Signaling", "DataRelay"]
        phone, disabled_ready = self.restart("android", "member", phone, disabled)
        assert not self.result(phone, "address")["mqtt"]
        assert not self.result(phone, "observe")["observations"]
        denied = phone.command("ping", address=self.relay_address(ar))
        assert not denied["ok"], denied
        phone, restored = self.restart("android", "member", phone, self.configs["android"])
        assert restored["node"] == pr["node"]
        assert self.result(phone, "ping", address=self.relay_address(ar))["reply"] == "PONG"
        self.receipt["disabled"] = {"passed": True, "failed_contact": denied, "same_node": restored["node"]}
        self.receipt["passed"] = True
        self.receipt["public_relay_tunnels"] = self.relay_proxy.snapshot()
        self.save()
        phone.close()
        admin.close()

    def cleanup(self):
        errors = []
        for process in self.processes:
            if process.proc.poll() is None:
                try:
                    process.close()
                except Exception as error:
                    errors.append(repr(error))
                    if process.proc.poll() is None:
                        process.proc.kill()
                        process.proc.wait(timeout=10)
        # A killed adb host is not proof that app_process stopped on the phone.
        # Match only this run's unique paths before terminating recorded PIDs.
        for process in self.processes:
            if hasattr(process, "remote_pid"):
                pid = str(process.remote_pid)
                command = self.shell("if [ -r /proc/" + pid + "/cmdline ]; then cat /proc/" + pid + "/cmdline; fi")
                if self.remote + "/libnode.so" in command or self.remote + "/node" in command:
                    self.shell("kill -CONT " + pid + "; kill -KILL " + pid)
        self.relay_proxy.close()
        self.shell("rm -rf " + shlex.quote(self.remote))
        removed = self.shell("test ! -e " + shlex.quote(self.remote) + " && echo removed").strip() == "removed"
        (self.output / "cleanup.json").write_text(json.dumps({"owned_device_directory_removed": removed, "close_errors": errors, "device_network_changed": False}))
        self.log.close()
        assert removed, "owned device directory was not cleaned"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--adb", default=str(pathlib.Path.home() / "Android/Sdk/platform-tools/adb"))
    parser.add_argument("--serial")
    parser.add_argument("--android-bin", default="target/aarch64-linux-android/debug/miopunch-native-probe")
    parser.add_argument("--android-lib", default="target/aarch64-linux-android/debug/libmiopunch_native_probe.so")
    parser.add_argument("--android-shell", required=True)
    parser.add_argument("--linux-bin", default="target/debug/miopunch-native-probe")
    parser.add_argument("--android-ip", required=True)
    parser.add_argument("--linux-ip", required=True)
    parser.add_argument("--broker", default="mqtt://broker.emqx.io:1883/")
    parser.add_argument("--stun-tcp", default="stun+tcp://turn.cloudflare.com:3478/")
    parser.add_argument("--stun-udp", default="stun://stun.cloudflare.com:3478/")
    parser.add_argument("--relay", default="https://aps1-1.relay.n0.iroh.link./")
    args = parser.parse_args()
    lab = PublicLab(args)
    try:
        lab.first_and_priority()
    except Exception as error:
        lab.receipt["error"] = repr(error)
        lab.save()
        raise
    finally:
        lab.receipt["public_relay_tunnels"] = lab.relay_proxy.snapshot()
        lab.save()
        lab.cleanup()
    print(json.dumps(lab.receipt))


if __name__ == "__main__":
    main()
