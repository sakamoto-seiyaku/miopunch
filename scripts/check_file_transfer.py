#!/usr/bin/env python3
"""Real public Native file operations using immutable SDK pins; never compiles.

Each OS generates its own source files. SSH carries commands, deployment and
private invitations only. Delivered file bytes travel over the admitted SDK
connection, and each receiving filesystem supplies an independent final hash.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import sys

from check_native_client import Host, Runner, quote


TERMINAL = {"complete", "failed", "cancelled"}


class FileRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.private_root = Path(tempfile.mkdtemp(prefix="m114-files-private-"))
        self.focus = args.focus
        self.qualifications = []
        root = Path(__file__).resolve().parents[1]
        sources = {Path(__file__).resolve()}
        for module in tuple(sys.modules.values()):
            name = getattr(module, "__file__", None)
            if name:
                path = Path(name).resolve()
                if path.suffix == ".py" and path.is_relative_to(root / "scripts"):
                    sources.add(path)
        if "check_browser_onboarding" in sys.modules:
            sources.add(Path(getattr(args, "harness", root / "browser-probe/harness/client-onboarding.mjs")).resolve())
        main = getattr(sys.modules["__main__"], "__file__", None)
        if main and Path(main).resolve().is_relative_to(root):
            sources.add(Path(main).resolve())
        self.controller_sources = {}
        for path in sorted(sources):
            contents = path.read_bytes()
            self.controller_sources[str(path)] = hashlib.sha256(contents).hexdigest()
            destination = self.output / "controller-source" / path.relative_to(root)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(contents)
        (self.output / "controller-sources.json").write_text(json.dumps(self.controller_sources, indent=2))

    def controller_drift(self):
        drift = []
        for name, expected in self.controller_sources.items():
            try:
                if hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected:
                    drift.append(name)
            except OSError:
                drift.append(name)
        return drift

    def path(self, name, remote):
        return self.remote_root + "/" + name if remote else str(self.private_root / name)

    def transfer(self, name, source_remote, destination_remote):
        # This method is exclusively for private invitation material.
        assert name.endswith("-invite.json")
        if source_remote != destination_remote:
            source = "laptop:" + self.path(name, True) if source_remote else self.path(name, False)
            destination = "laptop:" + self.path(name, True) if destination_remote else self.path(name, False)
            subprocess.run(["scp", "-q", source, destination], check=True)
        return self.path(name, destination_remote)

    def paired(self, prefix, creator_remote=False):
        creator = self.start(prefix + "-creator", creator_remote)
        creator.call("export_recovery_card", path=self.path(prefix + "-card.json", creator_remote))
        creator.call("acknowledge_recovery_saved")
        name = prefix + "-invite.json"
        creator.call("invite", path=self.path(name, creator_remote))
        join_remote = not creator_remote if not self.local_only else False
        invitation = self.transfer(name, creator_remote, join_remote)
        member = self.start(prefix + "-member", join_remote, "join", invitation)
        member.call("files_enable", ok=False, directory=self.path(prefix + "-premature-receive", join_remote))
        source, _ = self.generate(member, prefix + "-premature-source.bin", 11)
        member.call("file_send", ok=False, address=creator.address, path=source)
        assert member.call("files_status")["tasks"] == []
        self.remove(source, join_remote)
        self.record(member.label, "file_before_membership_rejected", dict(noTaskStarted=True, receiveServiceClosed=True))
        assert member.call("request", path=invitation)["stage"] == "AwaitingApproval"
        pending = creator.call("requests")["requests"]
        assert len(pending) == 1 and pending[0]["applicant"] == member.node
        creator.call("approve", **pending[0])
        assert member.call("request", path=invitation)["stage"] == "Complete"
        return creator, member

    def generate(self, host, name, size):
        """Generate a deterministic stream on the source OS, without copying it."""
        path = self.path(name, host.remote)
        if host.remote:
            script = "$p=" + quote(path) + "; $b=New-Object byte[] 65536; "
            script += "for($i=0;$i -lt $b.Length;$i++){$b[$i]=[byte]($i % 251)}; "
            script += "$f=[IO.File]::Open($p,[IO.FileMode]::CreateNew); try {$left=[int64]" + str(size)
            script += "; while($left -gt 0){$n=[int][Math]::Min($left,$b.Length); $f.Write($b,0,$n); $left-=$n}; $f.Flush($true)} finally {$f.Dispose()}; "
            script += "@{size=(Get-Item -LiteralPath $p).Length;sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower()} | ConvertTo-Json -Compress"
            value = json.loads(self.remote(script))
        else:
            block = bytes(index % 251 for index in range(65536))
            with open(path, "xb") as file:
                remaining = size
                while remaining:
                    chunk = block[:min(len(block), remaining)]
                    file.write(chunk)
                    remaining -= len(chunk)
            value = self.hash_file(path, False)
        assert value["size"] == size
        self.record(host.label, "source_generated_on_endpoint", dict(name=name, **value))
        return path, value

    def hash_file(self, path, remote):
        if remote:
            return json.loads(self.remote("$p=" + quote(path) + "; @{size=(Get-Item -LiteralPath $p).Length;sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower()} | ConvertTo-Json -Compress"))
        digest = hashlib.sha256()
        with open(path, "rb") as file:
            while chunk := file.read(1024 * 1024):
                digest.update(chunk)
        return dict(size=Path(path).stat().st_size, sha256=digest.hexdigest())

    def remove(self, path, remote, directory=False):
        if remote:
            self.remote("Remove-Item -LiteralPath " + quote(path) + (" -Recurse" if directory else ""))
        elif directory:
            shutil.rmtree(path)
        else:
            Path(path).unlink()

    def listing(self, directory, remote):
        if remote:
            value = self.remote("@(@(Get-ChildItem -Force -LiteralPath " + quote(directory)
                                + ") | ForEach-Object {$_.Name}) | ConvertTo-Json -Compress").strip()
            result = json.loads(value) if value else []
            return [result] if isinstance(result, str) else result
        return sorted(path.name for path in Path(directory).iterdir())

    def task(self, host, identifier):
        tasks = host.call("files_status")["tasks"]
        matches = [task for task in tasks if task["id"] == identifier]
        return matches[0] if matches else None

    def wait_task(self, host, identifier, terminal=True, progress=False):
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            task = self.task(host, identifier)
            if task and ((terminal and task["state"] in TERMINAL)
                         or (progress and task["bytes"] > 0 and task["state"] == "transferring")):
                return task
            time.sleep(0.02 if progress else 0.1)
        raise AssertionError("public file task did not reach the requested state")

    def enable(self, host, prefix):
        directory = self.path(prefix + "-received", host.remote)
        assert host.call("files_enable", directory=directory) == dict(enabled=True)
        return directory

    def delivered(self, sender, receiver, source, expected, directory, interrupt=False):
        identifier = sender.call("file_send", address=receiver.address, path=source)["task"]
        if interrupt:
            self.wait_task(receiver, identifier, terminal=False, progress=True)
            sender.call("file_disconnect", task=identifier)
        result = self.wait_task(sender, identifier)
        assert result["state"] == "complete", result
        received = self.wait_task(receiver, identifier)
        assert received["state"] == "complete", received
        assert result["receipt"] == received["receipt"]
        receipt = result["receipt"]
        assert receipt["size"] == expected["size"] and receipt["sha256"] == expected["sha256"]
        destination = directory + "/" + receipt["saved_name"]
        assert self.hash_file(destination, receiver.remote) == expected
        assert any(path["selected"] for path in result["transport"]["paths"])
        if interrupt:
            assert result["attempts"] >= 2 and result["resumed_bytes"] > 0, result
        assert not any(name.endswith(".partial") for name in self.listing(directory, receiver.remote))
        self.qualifications.append(dict(source=sender.label, receiver=receiver.label, **expected,
                                        resumed=interrupt, receipt=receipt, attempts=result["attempts"],
                                        resumed_bytes=result["resumed_bytes"], transport=result["transport"]))
        self.remove(destination, receiver.remote)
        return identifier

    def scenario(self, prefix, creator_remote=False):
        a, b = self.paired(prefix, creator_remote)
        if self.focus == "bounds":
            for source, receiver in [(a, b), (b, a)]:
                directory = self.enable(receiver, receiver.label)
                path, expected = self.generate(source, source.label + "-concurrent.bin", 64 * 1024**2)
                tasks = [source.call("file_send", address=receiver.address, path=path)["task"] for _ in range(4)]
                source.call("file_send", ok=False, address=receiver.address, path=path)
                for identifier in tasks:
                    sending = self.wait_task(source, identifier)
                    assert sending["state"] == "complete", sending
                    receiving = self.wait_task(receiver, identifier)
                    assert receiving["state"] == "complete" and receiving["receipt"] == sending["receipt"]
                    receipt = sending["receipt"]
                    destination = directory + "/" + receipt["saved_name"]
                    assert self.hash_file(destination, receiver.remote) == expected
                    self.remove(destination, receiver.remote)
                assert not self.listing(directory, receiver.remote)
                outsider = self.start(source.label + "-other-network", receiver.remote)
                outsider.call("export_recovery_card", path=self.path(outsider.label + "-card.json", outsider.remote))
                outsider.call("acknowledge_recovery_saved")
                other_directory = self.enable(outsider, outsider.label)
                denied = source.call("file_send", address=outsider.address, path=path)["task"]
                assert self.wait_task(source, denied)["state"] == "failed"
                assert outsider.call("files_status")["tasks"] == []
                assert not self.listing(other_directory, outsider.remote)
                outsider.call("files_disable")
                outsider.stop()
                self.record(source.label, "other_network_file_rejected", dict(noSavedFile=True, noReceiverTask=True))
                self.remove(path, source.remote)
                receiver.call("files_disable")
                self.record(source.label, "bounded_concurrent_tasks", dict(completed=4, fifthRejected=True,
                    independentTaskIds=len(set(tasks)), savedHashes=expected, activeLimit=4))
            for host in [a, b]:
                host.stop()
            return
        for source, receiver in [(a, b), (b, a)]:
            assert receiver.call("files_status")["enabled"] is False
            path, expected = self.generate(source, source.label + "-disabled.bin", 11)
            identifier = source.call("file_send", address=receiver.address, path=path)["task"]
            failed = self.wait_task(source, identifier)
            assert failed["state"] == "failed" and failed["error"] == "service_disabled", failed
            self.remove(path, source.remote)
            directory = self.enable(receiver, receiver.label)
            source.call("file_send", ok=False, address=receiver.address, path=self.path("absent", source.remote))
            sizes = [0, 11] if self.focus == "smoke" else [0, 11, 4 * 1024**2, 64 * 1024**2]
            for index, size in enumerate(sizes):
                path, expected = self.generate(source, source.label + f"-中文 space-{index}.bin", size)
                self.delivered(source, receiver, path, expected, directory)
                self.remove(path, source.remote)
            if self.focus != "smoke":
                path, expected = self.generate(source, source.label + "-continued.bin", 256 * 1024**2)
                self.delivered(source, receiver, path, expected, directory, interrupt=True)
                # Cancellation must stop a live receiver and clean its partial.
                identifier = source.call("file_send", address=receiver.address, path=path)["task"]
                self.wait_task(receiver, identifier, terminal=False, progress=True)
                source.call("file_cancel", task=identifier)
                assert self.wait_task(source, identifier)["state"] == "cancelled"
                assert self.wait_task(receiver, identifier)["state"] == "cancelled"
                assert not self.listing(directory, receiver.remote)
                self.remove(path, source.remote)
            receiver.call("files_disable")
            assert not receiver.call("files_status")["enabled"]
            # A real save failure after enabling cannot produce a completion.
            receiver.call("files_enable", directory=directory)
            self.remove(directory, receiver.remote, directory=True)
            path, expected = self.generate(source, source.label + "-save-failure.bin", 11)
            identifier = source.call("file_send", address=receiver.address, path=path)["task"]
            failed = self.wait_task(source, identifier)
            assert failed["state"] == "failed" and failed["error"] == "save_failed", failed
            assert failed["receipt"] is None
            self.remove(path, source.remote)
            receiver.call("files_disable")
        for host in [a, b]:
            status = host.call("files_status")
            assert status["chunk_bytes"] == 65536 and status["active_limit"] == 4 and status["history_limit"] == 64
            assert all(task["state"] in TERMINAL for task in status["tasks"])
            host.stop()

    def cleanup(self):
        results = super().cleanup()
        try:
            shutil.rmtree(self.private_root)
            results.append(dict(host="host", action="remove_owned_private_runtime", removed=True))
        except Exception as error:
            results.append(dict(host="host", action="remove_owned_private_runtime", error=type(error).__name__))
        if self.remote_owned and not any(item.get("remaining") for item in results):
            try:
                self.remote("Remove-Item -LiteralPath " + quote(self.remote_root) + " -Recurse")
                results.append(dict(host="laptop", action="remove_owned_private_runtime", removed=True))
            except Exception as error:
                results.append(dict(host="laptop", action="remove_owned_private_runtime", error=type(error).__name__))
        return results

    def run(self):
        passed = False
        try:
            self.deploy()
            self.scenario("linux-creates")
            if not self.local_only:
                self.scenario("windows-creates", creator_remote=True)
            passed = True
        finally:
            cleanup = self.cleanup()
            drift = self.controller_drift()
            passed = passed and not drift and all("error" not in item for item in cleanup)
            result = dict(passed=passed, qualifications=self.qualifications, cleanup=cleanup,
                          controller_sha256=self.controller_sources[str(Path(__file__).resolve())],
                          controllerSourceDrift=drift,
                          ssh_business_transfer=False, scope="local Linux only" if self.local_only else "actual Windows laptop and Linux host; available VPN/IP")
            (self.output / "result.json").write_text(json.dumps(result, indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path)
    parser.add_argument("--windows-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--local-only", action="store_true")
    parser.add_argument("--focus", choices=["smoke", "full", "bounds"], default="full")
    args = parser.parse_args()
    args.failure_only = False
    if not args.local_only and not (args.windows and args.windows_sha):
        parser.error("actual Windows pin and hash required")
    FileRunner(args).run()


if __name__ == "__main__":
    main()
