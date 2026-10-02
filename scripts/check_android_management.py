#!/usr/bin/env python3
"""Physical Android public CLI checks; independent identities and real TLS/SQLite."""
import argparse
import json
import pathlib
import shlex
import subprocess
import time
import uuid

from check_desktop_management import Process


class Lab:
    def __init__(self, args):
        self.args = args
        self.output = pathlib.Path(args.output).resolve()
        self.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        self.remote = "/data/local/tmp/mio53-" + uuid.uuid4().hex
        self.log = (self.output / "processes.log").open("w", buffering=1)
        self.processes = []
        self.adb = [args.adb] + (["-s", args.serial] if args.serial else [])
        self.shell("mkdir -m 700 " + self.remote)
        self.adb_run("push", args.android_bin, self.remote + "/node")
        self.shell("chmod 700 " + self.remote + "/node")
        environment = self.shell("getprop ro.product.model; getprop ro.build.version.release; getprop ro.build.version.sdk; uname -m")
        (self.output / "environment.txt").write_text(environment)

    def adb_run(self, *args):
        result = subprocess.run(self.adb + list(args), capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, (args[0], result.stderr)
        return result.stdout

    def shell(self, command):
        return self.adb_run("shell", command)

    def path(self, platform, name):
        return self.remote + "/" + name if platform == "android" else str(self.output / name)

    def command(self, platform, *args):
        if platform == "android":
            return self.adb + ["shell", shlex.join([self.remote + "/node", *args])]
        return [self.args.linux_bin, *args]

    def run(self, platform, *args):
        result = subprocess.run(self.command(platform, *args), capture_output=True, text=True, timeout=40)
        self.log.write(result.stdout + result.stderr)
        assert result.returncode == 0, (platform, result.returncode, result.stderr)
        return result.stdout

    def session(self, platform, name, mode="session"):
        args = ["network", mode, self.path(platform, name), "0.0.0.0"]
        command = self.command(platform, *args)
        if platform == "android":
            command[-1] = "echo MIOPID $$; exec " + command[-1]
        process = Process(command, self.log)
        process.platform = platform
        self.processes.append(process)
        if platform == "android":
            process.remote_pid = int(process.wait(lambda e: e.get("raw", "").startswith("MIOPID "))["raw"].split()[1])
        ready = process.wait(lambda e: e.get("event") == "ready")
        # Contact hints are untrusted reachability, never identity/authority.
        ip = self.args.android_ip if platform == "android" else self.args.linux_ip
        for addr in ready["address"]["addrs"]:
            if "Ip" in addr:
                addr["Ip"] = ip + ":" + addr["Ip"].rsplit(":", 1)[1]
        return process, ready

    def transfer(self, source, target, name, address):
        local = self.output / name
        if source == "android":
            self.adb_run("pull", self.path(source, name), str(local))
        data = json.loads(local.read_text())
        data["address"] = address
        local.write_text(json.dumps(data))
        local.chmod(0o600)
        if target == "android":
            self.adb_run("push", str(local), self.path(target, name))
            self.shell("chmod 600 " + shlex.quote(self.path(target, name)))

    def lan_profile(self, platform, name):
        # This foreground shell has no Android Java context for HTTPS trust.
        # Explicit controlled-LAN resources keep the runtime claim reproducible.
        profile = self.output / "lan-resources.json"
        profile.write_text("[]")
        if platform == "android":
            self.adb_run("push", str(profile), self.path(platform, profile.name))
        self.run(platform, "network", "resources-import", self.path(platform, name), self.path(platform, profile.name))

    def pair(self, creator, applicant, name, pause=False):
        owner, member = name + "-owner", name + "-member"
        invite = name + "-invite.json"
        self.run(creator, "network", "create", self.path(creator, owner))
        self.lan_profile(creator, owner)
        admin, ar = self.session(creator, owner)
        assert admin.command("invite", path=self.path(creator, invite))["ok"]
        self.transfer(creator, applicant, invite, ar["address"])
        self.run(applicant, "network", "join", self.path(applicant, member), self.path(applicant, invite))
        self.lan_profile(applicant, member)
        client, cr = self.session(applicant, member)
        assert client.command("request", path=self.path(applicant, invite))["result"]["stage"] == "AwaitingApproval"
        requests = admin.command("requests")["result"]["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == cr["node"]
        assert admin.command("approve", **requests[0])["ok"]
        result = client.command("request", path=self.path(applicant, invite), pause_before_save=pause)
        assert result["result"]["stage"] == ("PendingMaterials" if pause else "Complete"), result
        return admin, ar, client, cr, owner, member, invite, requests

    def first(self):
        for creator, applicant in ([(self.args.creator, "linux" if self.args.creator == "android" else "android")] if self.args.creator else [("android", "linux"), ("linux", "android")]):
            name = "first-" + creator
            admin, ar, client, cr, owner, member, invite, requests = self.pair(creator, applicant, name)
            assert client.command("ping", address=ar["address"])["result"]["reply"] == "PONG"
            target = self.run("linux", "identity", self.path("linux", name + "-target")).strip()
            assert admin.command("manage", action="appoint_administrator", target=cr["node"])["error"] == "elevation_required"
            assert admin.command("elevate", path=self.path(creator, owner + "/genesis.credential"), seconds=60)["ok"]
            assert admin.command("manage", action="appoint_administrator", target=cr["node"])["ok"]
            assert client.command("sync", address=ar["address"])["result"]["complete"]
            phone = admin if creator == "android" else client
            assert phone.command("manage", action="add_member", target=target)["ok"]
            assert phone.command("manage", action="remove_member", target=target)["ok"]
            assert admin.command("exit_elevation")["ok"]
            assert admin.command("manage", action="demote_administrator", target=cr["node"])["error"] == "elevation_required"
            client.close()
            admin.close()
        return {"case": "first-phone-create-join-daily-management-genesis", "passed": True}

    def kill(self, process):
        if process.platform == "android":
            self.shell("kill -KILL " + str(process.remote_pid))
        else:
            process.proc.kill()
        process.proc.wait(timeout=15)

    def restart(self):
        admin, ar, client, cr, owner, member, invite, requests = self.pair("linux", "android", "restart", pause=True)
        before = admin.command("status")["result"]["records"]
        config = client.command("resources")["result"]["config"]
        config["maintenance"]["exploration_interval_ms"] = 6000
        config["disabled_uses"] = ["Observation", "Lookup", "Signaling", "DataRelay"]
        assert client.command("save_resources", config=config)["ok"]
        self.kill(client)
        admin.close()
        admin, ar2 = self.session("linux", owner)
        client, cr2 = self.session("android", member)
        assert cr2["node"] == cr["node"] and ar2["node"] == ar["node"]
        assert client.command("resources")["result"]["config"] == config
        assert client.command("status")["result"]["stage"] == "PendingMaterials"
        assert admin.command("requests")["result"]["requests"] == requests
        refreshed = "restart-refreshed.json"
        assert admin.command("invite", path=self.path("linux", refreshed))["ok"]
        self.transfer("linux", "android", refreshed, ar2["address"])
        assert client.command("request", path=self.path("android", refreshed))["result"]["stage"] == "Complete"
        assert admin.command("status")["result"]["records"] == before
        assert client.command("ping", address=ar2["address"])["result"]["reply"] == "PONG"
        self.kill(client)
        client, cr3 = self.session("android", member)
        assert cr3["node"] == cr["node"]
        assert client.command("status")["result"]["stage"] == "Complete"
        assert client.command("ping", address=ar2["address"])["result"]["reply"] == "PONG"
        client.close()
        admin.close()
        return {"case": "sigkill-pending-materials-resources-and-complete-restart", "passed": True}

    def precode(self):
        owner, member, code = "precode-owner", "precode-member", "precode.json"
        self.run("android", "network", "create", self.path("android", owner))
        self.lan_profile("android", owner)
        phone, pr = self.session("android", owner)
        assert phone.command("issue_precode", path=self.path("android", code), seconds=3600, max_uses=2)["ok"]
        self.transfer("android", "linux", code, pr["address"])
        self.run("linux", "network", "join", self.path("linux", member), self.path("linux", code))
        self.lan_profile("linux", member)
        client, cr = self.session("linux", member)
        result = client.command("request", path=self.path("linux", code), pause_before_save=True)
        assert result["result"]["stage"] == "PendingMaterials", result
        assert phone.command("precode_status", path=self.path("android", code))["result"]["uses"] == 1
        before = phone.command("status")["result"]["records"]
        self.kill(phone)
        client.close()
        phone, pr2 = self.session("android", owner)
        client, cr2 = self.session("linux", member)
        assert pr2["node"] == pr["node"] and cr2["node"] == cr["node"]
        refreshed = "precode-refreshed.json"
        assert phone.command("refresh_invitation", path=self.path("android", code), destination=self.path("android", refreshed))["ok"]
        self.transfer("android", "linux", refreshed, pr2["address"])
        assert client.command("request", path=self.path("linux", refreshed))["result"]["stage"] == "Complete"
        assert phone.command("precode_status", path=self.path("android", code))["result"] == {"uses": 1, "count_trusted": True}
        assert phone.command("status")["result"]["records"] == before
        assert client.command("ping", address=pr2["address"])["result"]["reply"] == "PONG"
        client.close()
        phone.close()
        return {"case": "android-precode-persistent-count-pending-restart", "passed": True}

    def maintenance_entry(self):
        self.run("android", "network", "create", self.path("android", "entry"))
        self.lan_profile("android", "entry")
        phone, ready = self.session("android", "entry", "maintain")
        status = phone.command("status")
        assert status["ok"] and status["result"]["node"] == ready["node"], status
        assert phone.command("network_changed")["ok"]
        phone.close()
        return {"case": "shared-maintenance-platform-entry", "passed": True}

    def wait_status(self, process, predicate, seconds=50):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            response = process.command("status")
            assert response["ok"], response
            status = response["result"]
            if predicate(status):
                return status
            time.sleep(1)
        raise AssertionError(("status deadline", status))

    def maintained_pair(self, name):
        admin, ar, phone, pr, owner, member, _, _ = self.pair("linux", "android", name)
        phone.close()
        admin.close()
        admin, ar = self.session("linux", owner, "maintain")
        phone, pr = self.session("android", member, "maintain")
        assert phone.command("contact", address=ar["address"])["ok"]
        self.wait_status(phone, lambda s: s["state"] == "Connected" and s["last_sync"]["complete"])
        return admin, ar, phone, pr, owner, member

    def pause(self):
        admin, ar, phone, pr, owner, member = self.maintained_pair("pause")
        before = phone.command("status")["result"]
        suspended = phone.command("suspend")["result"]
        assert suspended["state"] == "Suspended" and not suspended["neighbors"]
        assert suspended["old_connections_ended"]
        assert not phone.command("network_changed")["ok"]
        resumed = phone.command("resume")["result"]
        assert resumed["node"] == pr["node"] and resumed["generation"] != pr["generation"]
        # Saved verified contacts recover without another contact command or approval.
        self.wait_status(phone, lambda s: s["state"] == "Connected")
        assert admin.command("contact", address=resumed["address"])["ok"]
        connected = self.wait_status(admin, lambda s: s["state"] == "Connected" and s["neighbors"])
        old_connection = connected["neighbors"][0]["connection"]
        self.shell("kill -STOP " + str(phone.remote_pid))
        try:
            failed = self.wait_status(admin, lambda s: s["state"] == "TemporarilyUnreachable" and not s["neighbors"])
            assert failed["generation"] == ar["generation"]
        finally:
            self.shell("kill -CONT " + str(phone.remote_pid))
        assert phone.command("network_changed")["ok"]
        assert admin.command("network_changed")["ok"]
        recovered = self.wait_status(admin, lambda s: s["state"] == "Connected" and s["neighbors"])
        assert recovered["neighbors"][0]["connection"] != old_connection
        self.wait_status(phone, lambda s: s["state"] == "Connected")
        self.kill(phone)
        phone, final = self.session("android", member, "maintain")
        assert final["node"] == pr["node"] and final["generation"] != resumed["generation"]
        status = self.wait_status(phone, lambda s: s["state"] == "Connected")
        assert status["records"] == before["records"]
        phone.close()
        admin.close()
        return {"case": "explicit-suspend-os-stop-and-kill-maintenance-recovery", "passed": True,
                "old_connection": old_connection, "new_connection": recovered["neighbors"][0]["connection"]}

    def network_change(self):
        assert "Wifi is enabled" in self.shell("cmd wifi status"), "Wi-Fi must initially be enabled"
        admin, ar, phone, pr, owner, member = self.maintained_pair("network")
        assert admin.command("contact", address=pr["address"])["ok"]
        connected = self.wait_status(admin, lambda s: s["state"] == "Connected" and s["neighbors"])
        old_connection = connected["neighbors"][0]["connection"]
        try:
            self.shell("svc wifi disable")
            assert phone.command("network_changed")["ok"]
            self.wait_status(admin, lambda s: s["state"] == "TemporarilyUnreachable" and not s["neighbors"])
        finally:
            self.shell("svc wifi enable")
        deadline = time.monotonic() + 45
        while self.args.android_ip not in self.shell("ip -4 addr show wlan0"):
            assert time.monotonic() < deadline, "Wi-Fi address did not recover"
            time.sleep(1)
        assert phone.command("network_changed")["ok"]
        assert admin.command("network_changed")["ok"]
        self.wait_status(phone, lambda s: s["state"] == "Connected")
        recovered = self.wait_status(admin, lambda s: s["state"] == "Connected" and s["neighbors"])
        assert recovered["neighbors"][0]["connection"] != old_connection
        assert phone.command("status")["result"]["node"] == pr["node"]
        phone.close()
        admin.close()
        return {"case": "physical-wifi-loss-reconnect-same-membership-new-connection", "passed": True,
                "wifi_restored": True}

    def cleanup(self):
        for process in self.processes:
            if process.proc.poll() is None:
                try:
                    process.close()
                except Exception:
                    self.kill(process)
        # Killing an adb host process alone need not kill its Android child.
        # Check only recorded PIDs whose current command still names our unique binary.
        for process in self.processes:
            if hasattr(process, "remote_pid"):
                pid = str(process.remote_pid)
                command = self.shell("if [ -r /proc/" + pid + "/cmdline ]; then cat /proc/" + pid + "/cmdline; fi")
                if self.remote + "/node" in command:
                    self.shell("kill -CONT " + pid + "; kill -KILL " + pid)
        self.shell("rm -rf " + shlex.quote(self.remote))
        assert self.shell("test ! -e " + shlex.quote(self.remote) + " && echo removed").strip() == "removed"
        (self.output / "cleanup.json").write_text(json.dumps({"owned_device_directory_removed": True}))
        self.log.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--adb", default=str(pathlib.Path.home() / "Android/Sdk/platform-tools/adb"))
    parser.add_argument("--serial")
    parser.add_argument("--creator", choices=["android", "linux"])
    parser.add_argument("--android-bin", default="target/aarch64-linux-android/debug/miopunch-native-probe")
    parser.add_argument("--linux-bin", default="target/debug/miopunch-native-probe")
    parser.add_argument("--android-ip", required=True)
    parser.add_argument("--linux-ip", required=True)
    parser.add_argument("--case", choices=["all", "first", "restart", "maintenance_entry", "pause", "network_change", "precode"], default="all")
    args = parser.parse_args()
    lab = Lab(args)
    results = []
    try:
        for name in ["first", "restart", "maintenance_entry", "pause", "network_change", "precode"] if args.case == "all" else [args.case]:
            results.append(getattr(lab, name)())
            (lab.output / "results.json").write_text(json.dumps(results, indent=2))
    except Exception as error:
        results.append({"passed": False, "error": str(error)})
        raise
    finally:
        (lab.output / "results.json").write_text(json.dumps(results, indent=2))
        lab.cleanup()
    print(json.dumps(results))


if __name__ == "__main__":
    main()
