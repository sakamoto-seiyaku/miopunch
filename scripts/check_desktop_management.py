#!/usr/bin/env python3
"""Exercise the public desktop CLI with real processes, storage and TLS/QUIC."""
import argparse
import json
import pathlib
import queue
import subprocess
import threading
import time


class Process:
    def __init__(self, command, log):
        self.log = log
        self.proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=log, text=True, bufsize=1)
        self.events = queue.Queue()
        self.sequence = 0
        def read():
            for line in self.proc.stdout:
                log.write(line)
                log.flush()
                try:
                    self.events.put(json.loads(line))
                except json.JSONDecodeError:
                    self.events.put({"raw": line.rstrip()})
            self.events.put({"eof": self.proc.poll()})
        threading.Thread(target=read, daemon=True).start()

    def wait(self, predicate, seconds=35):
        deadline = time.monotonic() + seconds
        while True:
            event = self.events.get(timeout=max(0.01, deadline - time.monotonic()))
            if predicate(event):
                return event
            assert "eof" not in event, event
            assert time.monotonic() < deadline, event

    def command(self, op, **args):
        self.sequence += 1
        self.proc.stdin.write(json.dumps({"id": self.sequence, "op": op, **args}) + "\n")
        self.proc.stdin.flush()
        return self.wait(lambda event: event.get("id") == self.sequence or event.get("error") == "invalid_command")

    def close(self):
        if self.proc.poll() is None:
            try:
                self.command("stop")
                self.proc.wait(timeout=20)
            except Exception:
                self.proc.kill()
                self.proc.wait(timeout=10)
        assert self.proc.returncode == 0, self.proc.returncode


class Lab:
    def __init__(self, args):
        self.args = args
        self.output = pathlib.Path(args.output).resolve()
        self.output.mkdir(parents=True, exist_ok=False, mode=0o700)
        self.log = (self.output / "processes.log").open("w", buffering=1)
        self.processes = []

    def path(self, platform, path):
        path = str(pathlib.Path(path).resolve())
        return "Z:" + path.replace("/", "\\") if platform == "windows" else path

    def command(self, platform):
        if platform == "windows":
            return [self.args.windows_runner, self.args.windows_bin]
        return [self.args.linux_bin]

    def run(self, platform, *args):
        result = subprocess.run(self.command(platform) + list(args), capture_output=True,
                                text=True, timeout=40)
        self.log.write(result.stdout + result.stderr)
        assert result.returncode == 0, (platform, args, result.returncode, result.stderr)
        return result.stdout

    def session(self, platform, directory):
        process = Process(self.command(platform) + ["network", "session", self.path(platform, directory)], self.log)
        self.processes.append(process)
        ready = process.wait(lambda event: event.get("event") == "ready")
        return process, ready

    def windows_roundtrip(self):
        directory = self.output / "raw-linux"
        process = Process(self.command("linux") + ["serve", "raw", str(directory)], self.log)
        self.processes.append(process)
        ready = process.wait(lambda event: event.get("raw", "").startswith("READY "))["raw"].split()
        reply = self.run("windows", "send", "raw", self.path("windows", self.output / "raw-windows"), ready[1], ready[2])
        assert "PONG" in reply, reply
        process.proc.wait(timeout=20)
        assert process.proc.returncode == 0
        return {"case": "windows-linux-raw-roundtrip", "passed": True}

    def restart_pending(self, creator_platform, applicant_platform, name):
        creator = self.output / (name + "-creator")
        applicant = self.output / (name + "-applicant")
        invitation = self.output / (name + "-invitation.json")
        self.run(creator_platform, "network", "create", self.path(creator_platform, creator))
        admin, ar = self.session(creator_platform, creator)
        assert admin.command("invite", path=self.path(creator_platform, invitation))["ok"]
        self.run(applicant_platform, "network", "join", self.path(applicant_platform, applicant), self.path(applicant_platform, invitation))
        member, mr = self.session(applicant_platform, applicant)
        assert member.command("request", path=self.path(applicant_platform, invitation))["result"]["stage"] == "AwaitingApproval"
        request = admin.command("requests")["result"]["requests"][0]
        assert admin.command("approve", **request)["ok"]
        result = member.command("request", path=self.path(applicant_platform, invitation), pause_before_save=True)
        assert result["result"]["stage"] == "PendingMaterials", result
        before = admin.command("status")["result"]
        member.close()
        admin.close()
        admin, reopened_admin = self.session(creator_platform, creator)
        member, reopened_member = self.session(applicant_platform, applicant)
        assert reopened_admin["node"] == ar["node"]
        assert reopened_member["node"] == mr["node"]
        assert member.command("status")["result"]["stage"] == "PendingMaterials"
        assert admin.command("requests")["result"]["requests"] == [request]
        updated_invitation = self.output / (name + "-updated-invitation.json")
        assert admin.command("invite", path=self.path(creator_platform, updated_invitation))["ok"]
        result = member.command("request", path=self.path(applicant_platform, updated_invitation))
        assert result["result"]["stage"] == "Complete", result
        assert admin.command("status")["result"]["records"] == before["records"]
        assert member.command("ping", address=reopened_admin["address"])["result"]["reply"] == "PONG"
        member.close()
        member, final = self.session(applicant_platform, applicant)
        assert final["node"] == mr["node"]
        assert member.command("status")["result"]["stage"] == "Complete"
        member.close()
        admin.close()
        return {"case": name, "creator": creator_platform, "applicant": applicant_platform, "passed": True}

    def management(self, creator_platform, applicant_platform, name):
        self.first(creator_platform, applicant_platform, name)
        creator = self.output / (name + "-creator")
        applicant = self.output / (name + "-applicant")
        admin, ar = self.session(creator_platform, creator)
        member, mr = self.session(applicant_platform, applicant)
        target = self.run("linux", "identity", str(self.output / (name + "-target"))).strip()
        result = member.command("manage", action="add_member", target=target)
        assert result.get("error") == "permission_denied", result
        result = admin.command("manage", action="appoint_administrator", target=mr["node"])
        assert result.get("error") == "elevation_required", result
        assert admin.command("manage", action="add_member", target=target)["ok"]
        credential = self.path(creator_platform, creator / "genesis.credential")
        assert admin.command("elevate", path=credential, seconds=60)["ok"]
        assert admin.command("manage", action="appoint_administrator", target=mr["node"])["ok"]
        assert member.command("sync", address=ar["address"])["result"]["complete"]
        assert {"node": mr["node"], "role": "admin"} in member.command("status")["result"]["members"]
        assert admin.command("manage", action="demote_administrator", target=mr["node"])["ok"]
        assert admin.command("exit_elevation")["ok"]
        assert admin.command("manage", action="appoint_administrator", target=mr["node"])["error"] == "elevation_required"
        assert member.command("sync", address=ar["address"])["result"]["complete"]
        assert {"node": mr["node"], "role": "member"} in member.command("status")["result"]["members"]
        assert admin.command("manage", action="remove_member", target=target)["ok"]
        assert admin.command("manage", action="remove_member", target=mr["node"])["ok"]
        assert member.command("ping", address=ar["address"])["error"] == "temporarily_unreachable"
        admin.close()
        member.close()
        admin, reopened = self.session(creator_platform, creator)
        assert reopened["node"] == ar["node"]
        assert all(entry["node"] != mr["node"] for entry in admin.command("status")["result"]["members"])
        assert admin.command("manage", action="appoint_administrator", target=mr["node"])["error"] == "elevation_required"
        admin.close()
        return {"case": name, "creator": creator_platform, "applicant": applicant_platform, "passed": True}

    def precode(self, creator_platform, applicant_platform, name):
        creator = self.output / (name + "-creator")
        applicant = self.output / (name + "-applicant")
        code = self.output / (name + "-code.json")
        self.run(creator_platform, "network", "create", self.path(creator_platform, creator))
        admin, ar = self.session(creator_platform, creator)
        result = admin.command("issue_precode", path=self.path(creator_platform, code), seconds=3600, max_uses=2)
        assert result["ok"], result
        self.run(applicant_platform, "network", "join", self.path(applicant_platform, applicant), self.path(applicant_platform, code))
        member, mr = self.session(applicant_platform, applicant)
        result = member.command("request", path=self.path(applicant_platform, code), pause_before_save=True)
        assert result["result"]["stage"] == "PendingMaterials", result
        assert admin.command("precode_status", path=self.path(creator_platform, code))["result"] == {"uses": 1, "count_trusted": True}
        before = admin.command("status")["result"]["records"]
        member.close()
        admin.close()
        admin, ar2 = self.session(creator_platform, creator)
        member, mr2 = self.session(applicant_platform, applicant)
        assert ar2["node"] == ar["node"] and mr2["node"] == mr["node"]
        refreshed = self.output / (name + "-refreshed.json")
        assert admin.command("refresh_invitation", path=self.path(creator_platform, code), destination=self.path(creator_platform, refreshed))["ok"]
        result = member.command("request", path=self.path(applicant_platform, refreshed))
        assert result["result"]["stage"] == "Complete", result
        assert admin.command("precode_status", path=self.path(creator_platform, code))["result"]["uses"] == 1
        assert admin.command("status")["result"]["records"] == before
        assert member.command("ping", address=ar2["address"])["result"]["reply"] == "PONG"
        member.close()
        admin.close()
        backup = self.output / (name + "-backup")
        restored = self.output / (name + "-restored")
        self.run(creator_platform, "network", "backup", self.path(creator_platform, creator), self.path(creator_platform, backup))
        self.run(creator_platform, "network", "restore", self.path(creator_platform, backup), self.path(creator_platform, restored))
        admin, restored_ready = self.session(creator_platform, restored)
        assert restored_ready["node"] == ar["node"]
        result = admin.command("precode_status", path=self.path(creator_platform, code))
        assert result["result"] == {"uses": 1, "count_trusted": False}, result
        old_restored = self.output / (name + "-old-restored.json")
        assert admin.command("refresh_invitation", path=self.path(creator_platform, code), destination=self.path(creator_platform, old_restored))["ok"]
        fresh = self.output / (name + "-fresh-member")
        self.run(applicant_platform, "network", "join", self.path(applicant_platform, fresh), self.path(applicant_platform, old_restored))
        member, _ = self.session(applicant_platform, fresh)
        result = member.command("request", path=self.path(applicant_platform, old_restored))
        assert result.get("error") == "delivery_incomplete", result
        assert member.command("status")["result"]["stage"] == "AwaitingApproval"
        assert admin.command("status")["result"]["records"] == before
        fresh_code = self.output / (name + "-fresh-code.json")
        assert admin.command("issue_precode", path=self.path(creator_platform, fresh_code), seconds=3600, max_uses=1)["ok"]
        assert member.command("request", path=self.path(applicant_platform, fresh_code))["result"]["stage"] == "Complete"
        assert admin.command("precode_status", path=self.path(creator_platform, fresh_code))["result"] == {"uses": 1, "count_trusted": True}
        member.close()
        admin.close()
        return {"case": name, "creator": creator_platform, "applicant": applicant_platform, "passed": True}

    def resources(self, creator_platform, applicant_platform, name):
        self.first(creator_platform, applicant_platform, name)
        creator = self.output / (name + "-creator")
        applicant = self.output / (name + "-applicant")
        admin, ar = self.session(creator_platform, creator)
        member, mr = self.session(applicant_platform, applicant)
        configurations = []
        for process in (admin, member):
            result = process.command("resources")
            assert result["ok"], result
            config = result["result"]["config"]
            assert len(config["defaults"]["resources"]) > 0
            config["replacement"] = []
            config["disabled_uses"] = ["Observation", "Lookup", "Signaling", "DataRelay"]
            config["tcp_direct_disabled"] = True
            result = process.command("save_resources", config=config)
            assert result["result"]["effective_on_restart"], result
            configurations.append(config)
        member.close()
        admin.close()
        admin, ar2 = self.session(creator_platform, creator)
        member, mr2 = self.session(applicant_platform, applicant)
        assert ar2["node"] == ar["node"] and mr2["node"] == mr["node"]
        for process, ready, expected in [(admin, ar2, configurations[0]), (member, mr2, configurations[1])]:
            assert process.command("resources")["result"]["config"] == expected
            assert not any("Relay" in address for address in ready["address"]["addrs"])
        assert member.command("ping", address=ar2["address"])["result"]["reply"] == "PONG"
        admin.close()
        result = member.command("ping", address=ar2["address"])
        assert result.get("error") == "temporarily_unreachable", result
        start = time.monotonic()
        assert member.command("ping", address=ar2["address"])["error"] == "temporarily_unreachable"
        assert time.monotonic() - start < 2, "retry must use the core backoff"
        assert member.command("status")["result"]["stage"] == "Complete"
        admin, ar3 = self.session(creator_platform, creator)
        time.sleep(1.1)
        assert member.command("ping", address=ar3["address"])["result"]["reply"] == "PONG"
        member.close()
        admin.close()
        return {"case": name, "creator": creator_platform, "applicant": applicant_platform, "passed": True}

    def first(self, creator_platform, applicant_platform, name):
        creator = self.output / (name + "-creator")
        applicant = self.output / (name + "-applicant")
        invitation = self.output / (name + "-invitation.json")
        self.run(creator_platform, "network", "create", self.path(creator_platform, creator))
        admin, ar = self.session(creator_platform, creator)
        assert admin.command("invite", path=self.path(creator_platform, invitation))["ok"]
        self.run(applicant_platform, "network", "join", self.path(applicant_platform, applicant), self.path(applicant_platform, invitation))
        member, mr = self.session(applicant_platform, applicant)
        assert ar["node"] != mr["node"]
        result = member.command("request", path=self.path(applicant_platform, invitation))
        assert result["result"]["stage"] == "AwaitingApproval", result
        requests = admin.command("requests")["result"]["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == mr["node"], requests
        assert admin.command("approve", **requests[0])["ok"]
        result = member.command("request", path=self.path(applicant_platform, invitation))
        assert result["result"]["stage"] == "Complete", result
        result = member.command("ping", address=ar["address"])
        assert result["result"]["reply"] == "PONG", result
        member.close()
        admin.close()
        return {"case": name, "creator": creator_platform, "applicant": applicant_platform, "passed": True}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--linux-bin", default="target/debug/miopunch-native-probe")
    parser.add_argument("--windows-bin", default="target/x86_64-pc-windows-gnu/debug/miopunch-native-probe.exe")
    parser.add_argument("--windows-runner")
    parser.add_argument("--case", choices=["all", "first", "restart", "management", "precode", "resources"], default="all")
    args = parser.parse_args()
    lab = Lab(args)
    results = []
    try:
        if args.windows_runner:
            results.append(lab.windows_roundtrip())
        cases = {"first": lab.first, "restart": lab.restart_pending, "management": lab.management,
                 "precode": lab.precode, "resources": lab.resources}
        for name in cases if args.case == "all" else [args.case]:
            case = cases[name]
            if args.windows_runner:
                results.append(case("linux", "windows", name + "-linux-windows"))
                results.append(case("windows", "linux", name + "-windows-linux"))
            else:
                results.append(case("linux", "linux", name + "-linux-development-only"))
    except Exception as error:
        results.append({"passed": False, "error": str(error)})
        raise
    finally:
        for process in lab.processes:
            if process.proc.poll() is None:
                try:
                    process.close()
                except Exception:
                    if process.proc.poll() is None:
                        process.proc.kill()
                        process.proc.wait(timeout=10)
        (lab.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        lab.log.close()
    print(json.dumps(results))


if __name__ == "__main__":
    main()
