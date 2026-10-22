#!/usr/bin/env python3
"""Exercise pinned NativeClient hosts; SSH carries control, never member data.

No compilation occurs here. Invitations are private capabilities and are never
included in the public operation transcript. Native PING/PONG bytes are produced
inside the SDK host, not supplied by this runner or transported by SSH.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading
import time


def ps(script):
    encoded = base64.b64encode(("$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new(); " + script).encode("utf-16le")).decode()
    return ["ssh", "-oBatchMode=yes", "-oConnectTimeout=12", "laptop",
            "powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive",
            "-EncodedCommand", encoded]


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


class Host:
    def __init__(self, runner, label, remote, action, directory, invitation=None):
        self.runner, self.label, self.remote = runner, label, remote
        self.directory = directory
        args = [action, directory] + ([invitation] if invitation else [])
        if remote:
            command = ps("[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); & "
                         + quote(runner.remote_binary) + " " + " ".join(map(quote, args)) + "; exit $LASTEXITCODE")
        else:
            command = [str(runner.linux), *args]
        self.stderr = (runner.output / (label + ".stderr")).open("w")
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True, encoding="utf-8")
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        runner.hosts.append(self)
        self.ready = self.receive()
        assert self.ready.get("event") == "ready", self.ready
        self.node = self.ready["status"]["node"]
        self.address = self.ready["address"]
        runner.record(label, "ready", self.ready)

    def read(self):
        for line in self.process.stdout:
            if line.strip():
                self.lines.put(line)
        self.lines.put(None)

    def receive(self):
        deadline = time.monotonic() + 90
        while True:
            line = self.lines.get(timeout=max(0, deadline - time.monotonic()))
            if line is None:
                raise RuntimeError(f"{self.label} exited before response; inspect stderr")
            value = json.loads(line)
            if value.get("event") and value.get("event") != "ready":
                self.runner.record(self.label, "sdk_event", value)
            else:
                return value

    def call(self, op, ok=True, **fields):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        self.process.stdin.write(json.dumps(dict(id=identifier, op=op, **fields)) + "\n")
        self.process.stdin.flush()
        answer = self.receive()
        self.runner.record(self.label, op, answer)
        assert answer["id"] == identifier and answer["ok"] == ok, answer
        return answer.get("result")

    def reject_malformed_commands(self):
        before = self.call("status")
        for line in ["{", "x" * 65537, '{"id":0,"op":"unknown"}']:
            self.process.stdin.write(line + "\n")
            self.process.stdin.flush()
            answer = self.receive()
            assert answer == dict(id=None, ok=False, error="invalid_command")
            self.runner.record(self.label, "malformed_command", answer)
        assert self.call("status") == before

    def stop(self):
        if self.process.poll() is None:
            self.call("stop")
            self.process.stdin.close()
        code = self.process.wait(timeout=20)
        self.stderr.close()
        assert code == 0, (self.label, code)


class Runner:
    def __init__(self, args):
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.linux = args.linux.resolve()
        self.windows = args.windows.resolve() if args.windows else None
        self.remote_root = "C:/Users/stati/AppData/Local/Temp/" + self.output.name
        self.remote_binary = self.remote_root + "/miopunch-client.exe"
        self.sequence, self.hosts, self.events = 0, [], []
        self.local_only = args.local_only
        self.failure_only = args.failure_only
        self.remote_owned = False
        for binary, expected in [(self.linux, args.linux_sha), (self.windows, args.windows_sha)]:
            if binary:
                assert hashlib.sha256(binary.read_bytes()).hexdigest() == expected
        self.record("runner", "pins", {"linux": str(self.linux), "linux_sha": args.linux_sha,
                                        "windows": str(self.windows), "windows_sha": args.windows_sha})

    def record(self, host, op, answer):
        event = dict(host=host, op=op, answer=answer, monotonic=time.monotonic())
        self.events.append(event)
        with (self.output / "events.jsonl").open("a") as out:
            out.write(json.dumps(event) + "\n")

    def remote(self, script):
        return subprocess.check_output(ps("$ErrorActionPreference='Stop'; " + script), text=True, timeout=60)

    def deploy(self):
        if not self.local_only:
            self.remote("New-Item -ItemType Directory -Path " + quote(self.remote_root) + " | Out-Null")
            self.remote_owned = True
            subprocess.run(["scp", "-q", str(self.windows), "laptop:" + self.remote_binary], check=True)
            actual = self.remote("(Get-FileHash -Algorithm SHA256 " + quote(self.remote_binary) + ").Hash").strip().lower()
            assert actual == hashlib.sha256(self.windows.read_bytes()).hexdigest()
            self.record("laptop", "deployment", {"sha256": actual,
                "os": self.remote("[Environment]::OSVersion.VersionString").strip()})

    def invalid_invitation(self, remote):
        name = "invalid-invite.json"
        local = self.output / name
        local.write_text("{}")
        local.chmod(0o600)
        invitation = self.transfer(name, False, remote)
        directory = self.path("invalid-state", remote)
        args = ["join", directory, invitation]
        command = ps("& " + quote(self.remote_binary) + " " + " ".join(map(quote, args))
                     + "; exit $LASTEXITCODE") if remote else [str(self.linux), *args]
        result = subprocess.run(command, capture_output=True, timeout=30)
        assert result.returncode != 0 and not result.stdout
        exists = self.remote("Test-Path -LiteralPath " + quote(directory)).strip() == "True" if remote else Path(directory).exists()
        assert not exists
        self.record("laptop" if remote else "host", "invalid_invitation",
                    dict(exit=result.returncode, state_created=exists))

    def path(self, name, remote):
        return self.remote_root + "/" + name if remote else str(self.output / name)

    def transfer(self, name, source_remote, destination_remote):
        if source_remote == destination_remote:
            return self.path(name, source_remote)
        local = self.output / name
        if source_remote:
            subprocess.run(["scp", "-q", "laptop:" + self.path(name, True), str(local)], check=True)
            local.chmod(0o600)
        else:
            subprocess.run(["scp", "-q", str(local), "laptop:" + self.path(name, True)], check=True)
        return self.path(name, destination_remote)

    def start(self, label, remote, action="create", invitation=None):
        return Host(self, label, remote, action, self.path(label + "-state", remote), invitation)

    def reopen(self, old):
        before = old.call("status")
        old.stop()
        host = Host(self, old.label + "-reopen", old.remote, "open", old.directory)
        assert host.node == old.node
        assert host.ready["status"] == before
        return host

    def exchange(self, a, b):
        for source, target in [(a, b), (b, a)]:
            result = source.call("ping", address=target.address)
            assert result["reply"] == "PONG"
            assert any(p["selected"] for p in result["transport"]["paths"])

    def scenario(self, creator_remote, prefix):
        join_remote = not creator_remote if not self.local_only else False
        creator = self.start(prefix + "-creator", creator_remote)
        name = prefix + "-invite.json"
        creator.call("invite", path=self.path(name, creator_remote))
        invitation = self.transfer(name, creator_remote, join_remote)
        joiner = self.start(prefix + "-joiner", join_remote, "join", invitation)
        assert creator.node != joiner.node
        assert creator.ready["status"]["network"] == joiner.ready["status"]["network"]
        assert joiner.ready["status"]["stage"] != "Complete"
        joiner.call("ping", ok=False, address=creator.address)
        time.sleep(2)  # SDK contact backoff applies across protocols.
        assert joiner.call("request", path=invitation)["stage"] == "AwaitingApproval"
        requests = creator.call("requests")["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == joiner.node
        joiner.call("request", path=invitation)
        assert creator.call("requests")["requests"] == requests
        creator.call("approve", **requests[0])
        before = creator.call("status")["records"]
        assert joiner.call("request", path=invitation, authorization_only=True)["stage"] == "PendingMaterials"
        joiner.call("ping", ok=False, address=creator.address)
        joiner = self.reopen(joiner)
        assert joiner.ready["status"]["stage"] == "PendingMaterials"
        assert joiner.call("request", path=invitation)["stage"] == "Complete"
        assert joiner.call("request", path=invitation)["stage"] == "Complete"
        assert creator.call("status")["records"] == before
        self.exchange(creator, joiner)
        joiner = self.reopen(joiner)
        creator = self.reopen(creator)
        assert joiner.ready["status"]["stage"] == "Complete"
        self.exchange(creator, joiner)
        name = prefix + "-precode.json"
        creator.call("issue_precode", path=self.path(name, creator_remote), seconds=3600, max_uses=1)
        code = self.transfer(name, creator_remote, join_remote)
        applicant = self.start(prefix + "-precode", join_remote, "join", code)
        assert applicant.node not in [creator.node, joiner.node]
        assert applicant.call("request", path=code, authorization_only=True)["stage"] == "PendingMaterials"
        assert creator.call("precode_status", path=self.path(name, creator_remote))["uses"] == 1
        before = creator.call("status")["records"]
        applicant = self.reopen(applicant)
        assert applicant.ready["status"]["stage"] == "PendingMaterials"
        assert applicant.call("request", path=code)["stage"] == "Complete"
        assert applicant.call("request", path=code)["stage"] == "Complete"
        assert creator.call("precode_status", path=self.path(name, creator_remote))["uses"] == 1
        assert creator.call("status")["records"] == before
        self.exchange(creator, applicant)
        applicant = self.reopen(applicant)
        assert applicant.ready["status"]["stage"] == "Complete"
        self.exchange(creator, applicant)
        for host in [applicant, joiner, creator]:
            host.stop()

    def cleanup(self):
        """Attempt every owned cleanup independently and retain private state."""
        results = []
        for host in self.hosts:
            try:
                host.stop()
                results.append(dict(host=host.label, exit=0))
            except Exception as error:
                results.append(dict(host=host.label, error=type(error).__name__))
                try:
                    if host.process.poll() is None:
                        host.process.kill()
                    host.process.wait(timeout=10)
                    host.stderr.close()
                except Exception as kill_error:
                    results.append(dict(host=host.label, action="force_stop", error=type(kill_error).__name__))
        # A failed local cleanup must never prevent checking remote ownership.
        if self.remote_owned:
            try:
                script = "$exe=[IO.Path]::GetFullPath(" + quote(self.remote_binary) + "); "
                script += "$owned=@(Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -eq $exe}); "
                script += "$owned | ForEach-Object {Stop-Process -Id $_.ProcessId -Force}; "
                script += "@{forced=$owned.Count; remaining=@(Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -eq $exe}).Count} | ConvertTo-Json -Compress"
                remote_cleanup = json.loads(self.remote(script))
                self.record("laptop", "cleanup", remote_cleanup)
                result = dict(host="laptop", action="owned_process_check", **remote_cleanup)
                if remote_cleanup["forced"] or remote_cleanup["remaining"]:
                    result["error"] = "native_process_did_not_exit"
                results.append(result)
            except Exception as error:
                results.append(dict(host="laptop", action="owned_process_check", error=type(error).__name__))
            try:
                self.remote("Get-ChildItem -LiteralPath " + quote(self.remote_root)
                            + " -Filter '*.json' | Remove-Item; if (Test-Path -LiteralPath "
                            + quote(self.remote_binary) + ") {Remove-Item -LiteralPath " + quote(self.remote_binary) + "}")
            except Exception as error:
                results.append(dict(host="laptop", action="remove_deployment", error=type(error).__name__))
        # Bearer files are removed; durable state/raw evidence remain private.
        for path in [*self.output.glob("*-invite.json"), *self.output.glob("*-precode.json")]:
            try:
                path.unlink()
            except Exception as error:
                results.append(dict(host="host", action="remove_invitation", error=type(error).__name__))
        return results

    def run(self):
        passed = False
        try:
            self.deploy()
            self.invalid_invitation(False)
            if not self.local_only:
                self.invalid_invitation(True)
            if self.failure_only:
                for remote in ([False] if self.local_only else [False, True]):
                    host = self.start("input-windows" if remote else "input-linux", remote)
                    host.reject_malformed_commands()
                    self.reopen(host).stop()
            else:
                self.scenario(False, "linux-creates")
                if not self.local_only:
                    self.scenario(True, "windows-creates")
            passed = True
        finally:
            cleanup = self.cleanup()
            passed = passed and all("error" not in entry for entry in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed, cleanup=cleanup), indent=2))
        assert passed
        assert all("error" not in entry for entry in cleanup)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--local-only", action="store_true")
    parser.add_argument("--failure-only", action="store_true", help="Only invitation/control-input failures and reopen/cleanup")
    args = parser.parse_args()
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("Windows pin and hash required for two-device acceptance")
    Runner(args).run()


if __name__ == "__main__":
    main()
