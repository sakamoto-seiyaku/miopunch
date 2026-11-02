#!/usr/bin/env python3
"""Current Windows/Linux forwarding through actual independent OS sockets."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import queue
import socket
import subprocess
import threading
import time
import uuid

from check_forwarding import Node, Runner, Target
from check_native_client import ps, quote


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class WindowsNode:
    def __init__(self, runner, invitation):
        self.runner, self.label = runner, "windows"
        self.root = "C:/Users/stati/AppData/Local/Temp/m118-" + uuid.uuid4().hex
        self.directory = self.root.replace('/', '\\') + '\\runtime'
        self.process = self.reader = None
        self.remote_owned = False
        self.remote_terminal = False
        self.mail_sequence = 0
        self.queue = queue.Queue(maxsize=128)
        self.reader_error = None
        runner.nodes.append(self)
        self.remote("if(Test-Path -LiteralPath " + quote(self.root) + "){throw 'fresh root required'}; "
                    "if((Get-FileHash -LiteralPath " + quote(runner.windows_sdk)
                    + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne " + quote(runner.windows_sha)
                    + "){throw 'SDK pin mismatch'}; New-Item -ItemType Directory -Path "
                    + quote(self.root) + " | Out-Null")
        self.remote_owned = True
        self.remote("New-Item -ItemType HardLink -Path " + quote(self.root + "/miopunch-client.exe")
                    + " -Value " + quote(runner.windows_sdk) + " | Out-Null")
        source = Path(__file__).resolve().parent
        for name in ["windows_forwarding_host.ps1", "windows_forwarding_sockets.cs"]:
            runner.freeze_source(source / name)
        manifest = runner.private / "windows-manifest.json"
        manifest.write_text(json.dumps({"files": {"exe": runner.windows_sha,
            "host": digest(source / "windows_forwarding_host.ps1"),
            "helper": digest(source / "windows_forwarding_sockets.cs")}}))
        for path, name in [(manifest, "manifest.json"), (invitation, "invitation.json"),
                           (source / "windows_forwarding_host.ps1", "windows_forwarding_host.ps1"),
                           (source / "windows_forwarding_sockets.cs", "windows_forwarding_sockets.cs")]:
            result = subprocess.run(["scp", "-q", str(path), "laptop:" + self.root + "/" + name],
                                    capture_output=True, text=True, timeout=30)
            assert result.returncode == 0, "owned control transfer failed"
            self.remote("if((Get-FileHash -LiteralPath " + quote(self.root + "/" + name)
                        + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne " + quote(digest(path))
                        + "){throw 'deployed source mismatch'}")
        self.stderr = (runner.output / "windows.stderr").open("w")
        command = "& " + quote(self.root + "/windows_forwarding_host.ps1") + " -Manifest " + quote(self.root + "/manifest.json")
        command += " -RunRoot " + quote(self.directory) + " -Mode join -Invitation " + quote(self.root + "/invitation.json")
        command += " -Mailbox"
        if getattr(runner, "windows_terminal_consumer", False):
            command += " -TerminalConsumer"
        command += "; exit $LASTEXITCODE"
        (runner.output / "windows-invocation.json").write_text(json.dumps({"root": self.root, "command": command}, indent=2))
        self.process = subprocess.Popen(ps(command), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True)
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        ready = self.receive()
        assert ready.get("event") == "ready", ready
        self.run_id = ready["run_id"]
        self.identity, self.address = ready["sdk"]["status"]["node"], ready["sdk"]["address"]
        self.targets = {protocol: ready[protocol + "_target"] for protocol in ["tcp", "udp"]}
        runner.record(self.label, "ready", ready)
        assert self.call("request", path=self.directory + "\\invitation.json")["stage"] == "Complete"

    def remote(self, script):
        value = subprocess.run(ps("$ErrorActionPreference='Stop'; " + script), capture_output=True, text=True, timeout=30)
        assert value.returncode == 0, "remote controller failed: " + value.stderr[:300]
        return value.stdout

    def read(self):
        try:
            for line in self.process.stdout:
                if line.strip():
                    value = json.loads(line)
                    with (self.runner.output / "windows.stdout.jsonl").open("a") as log:
                        log.write(json.dumps(value) + "\n")
                    self.queue.put(value, timeout=2)
        except BaseException as failure:
            self.reader_error = type(failure).__name__
        finally:
            try:
                self.queue.put(None, timeout=2)
            except queue.Full:
                self.reader_error = "QueueFull"

    def receive(self):
        value = self.queue.get(timeout=90)
        assert value is not None, "Windows technical host exited before response"
        return value

    def control(self, kind, **fields):
        self.runner.sequence += 1
        identifier = self.runner.sequence
        self.send(dict(id=identifier, kind=kind, **fields))
        self.runner.record(self.label, "control_sent", {"id": identifier, "kind": kind})
        value = self.receive()
        self.runner.record(self.label, kind, value)
        assert value.get("id") == identifier and value.get("ok"), value
        return value["result"]

    def send(self, packet):
        # The SSH/Console stdin path lost sequential requests. Each control
        # packet now becomes visible only after its complete atomic publication.
        body = json.dumps(packet).encode()
        assert len(body) <= 65536
        self.mail_sequence += 1
        assert self.mail_sequence <= 4096
        path = self.directory + '/requests/%06d.json' % self.mail_sequence
        self.remote("[IO.File]::WriteAllBytes(" + quote(path + '.pending')
                    + ",[Convert]::FromBase64String(" + quote(base64.b64encode(body).decode())
                    + ")); [IO.File]::Move(" + quote(path + '.pending') + "," + quote(path) + ")")

    def call(self, operation, /, ok=True, **fields):
        value = self.control("sdk", command=dict(op=operation, **fields))
        assert value["ok"] is ok, value
        return value.get("result")

    def close(self):
        errors, final = [], None
        if self.process is not None:
            try:
                if self.process.poll() is None:
                    self.send({"kind": "close", "id": 999999998})
                self.process.wait(timeout=40)
                self.reader.join(timeout=3)
                assert not self.reader.is_alive() and self.reader_error is None
                self.process.stdout.close()
                self.stderr.close()
                final = json.loads(self.remote("[IO.File]::ReadAllText(" + quote(self.root + "/runtime.result.json") + ")"))
                owner = self.remote("[IO.File]::ReadAllText(" + quote(self.root + "/runtime-owner.jsonl") + ")")
                (self.runner.output / "windows-owner.jsonl").write_text(owner)
                (self.runner.output / "windows-final.json").write_text(json.dumps(final, indent=2))
                self.remote_terminal = final["all_owned_terminal"]
                assert final["run_id"] == self.run_id and final["private_removed"]
                assert not final["failures"] and self.process.returncode == 0
            except BaseException as failure:
                errors.append(type(failure).__name__)
        if self.remote_owned and self.terminal():
            try:
                assert final is None or final["private_removed"]
                self.remote("Remove-Item -LiteralPath " + quote(self.root) + " -Recurse")
            except BaseException as failure:
                errors.append(type(failure).__name__)
        self.cleanup = {"node": self.label, "terminal": self.terminal(), "errors": errors, "final": final}
        assert not errors and self.terminal(), self.cleanup
        return self.cleanup

    def terminal(self):
        return self.process is None or (self.process.poll() is not None and not self.reader.is_alive() and self.remote_terminal)


class PlatformRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.freeze_source(__file__)
        self.freeze_source(Path(__file__).with_name("check_native_client.py"))
        self.windows_sdk, self.windows_sha = args.windows_sdk, args.windows_sha
        self.reuse_windows_tcp = args.reuse_windows_tcp
        if self.reuse_windows_tcp:
            previous = self.reuse_windows_tcp.resolve()
            events = [json.loads(line) for line in (previous / 'events.jsonl').read_text().splitlines()]
            proof = [e for e in events if e['operation'] == 'windows_provider_tcp']
            owner = json.loads((previous / 'windows-owner.jsonl').read_text().splitlines()[0])
            assert owner['owner']['exe_sha256'] == self.windows_sha
            assert any(e['operation'] == 'pin' and e['result']['sha256'] == args.sha256 for e in events)
            assert len(proof) == 1 and proof[0]['result'] == {
                'independent_target_exact': True, 'reply_exact': True, 'length': 257}
            self.record('controller', 'reuse_windows_provider_tcp', {'path': str(previous),
                        'events_sha256': digest(previous / 'events.jsonl'), 'proof': proof[0]})
        self.native_configuration = self.private / "resources.json"
        self.native_configuration.write_text(json.dumps({"defaults": {"revision": 2, "resources": []},
            "replacement": [], "disabled": [], "disabled_uses": ["Observation", "Lookup", "Signaling", "DataRelay"],
            "lan_discovery": "Disabled", "port_mapping": "Disabled"}))

    def scenario(self):
        linux = Node(self, "linux")
        invitation = self.private / "windows-invitation.json"
        linux.call("issue_precode", path=str(invitation), seconds=1800, max_uses=1)
        windows = WindowsNode(self, invitation)
        linux.call("service_sync", address=windows.address)
        try:
            self.exchanges(linux, windows)
        except BaseException:
            self.record('controller', 'target_observations_at_failure', [
                {'protocol': target.protocol, 'chunks': len(target.received),
                 'bytes': sum(len(data) for data in target.received), 'errors': target.errors}
                for target in self.targets])
            for node in [windows, linux]:
                try:
                    state = node.call('forward_status')
                    self.record(node.label, 'forward_status_at_failure', state)
                except BaseException as failure:
                    self.record(node.label, 'forward_observation_failed', {'type': type(failure).__name__})
            raise

    def exchanges(self, linux, windows):
        for protocol in ["tcp", "udp"]:
            length, seed = 257, 19
            payload = bytes((index + seed) % 251 for index in range(length))
            if not (protocol == 'tcp' and self.reuse_windows_tcp):
                # Windows provider; actual Windows target sees exactly this data.
                assert windows.control("target_probe", protocol=protocol, length=length, seed=seed)["exact"]
                windows.control("expect", protocol=protocol, length=length, seed=seed)
                windows.call("forward_enable", protocol=protocol, targets=[windows.targets[protocol]])
                forward = linux.call("forward_start", address=windows.address, protocol=protocol,
                                     target=windows.targets[protocol], bind="[::1]:0")
                host, port = forward["listen"].rsplit(':', 1)
                with socket.socket(socket.AF_INET6, socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM) as client:
                    client.settimeout(75)
                    client.connect((host.strip('[]'), int(port)))
                    client.sendall(payload) if protocol == "tcp" else client.send(payload)
                    response = b''
                    while len(response) < length:
                        part = client.recv(65536)
                        assert part, "forwarded reply ended early"
                        response += part
                        if protocol == "udp":
                            break
                    assert response == payload
                observed = windows.control("target_status")
                assert observed[protocol + "_case_exact"]
                assert observed["tcp_case_bytes"] == length if protocol == "tcp" else observed["udp_case_datagrams"] == 1
                linux.call("forward_stop", task=forward["id"])
                self.record("pair", "windows_provider_" + protocol, {"independent_target_exact": True, "reply_exact": True, "length": length})
            # Linux provider; the independent Windows client uses its real OS socket.
            target = Target(self, protocol, prefix=b'')
            linux.call("forward_enable", protocol=protocol, targets=[target.address])
            forward = windows.call("forward_start", address=linux.address, protocol=protocol,
                                   target=target.address, bind="[::1]:0")
            client_id = windows.control("client_open", protocol=protocol, address=forward["listen"])["client"]
            reply = windows.control("client_exchange", client=client_id, length=length, seed=seed)
            assert reply["exact"] and reply["length"] == length
            assert b''.join(target.received) == payload
            if protocol == "udp":
                assert target.received == [payload]
            windows.control("client_close", client=client_id)
            windows.call("forward_stop", task=forward["id"])
            self.record("pair", "linux_provider_" + protocol, {"independent_target_exact": True, "reply_exact": True, "length": length})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", required=True)
    parser.add_argument("--windows-sdk", required=True)
    parser.add_argument("--windows-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reuse-windows-tcp", type=Path)
    args = parser.parse_args()
    args.case, args.loopback = "platform", "::1"
    PlatformRunner(args).run()


if __name__ == "__main__":
    main()
