#!/usr/bin/env python3
"""Fresh private tmux plus public terminal operations; consumes immutable pins."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from check_forwarding import isolated_environment
from check_web_service import OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources, identity
from web_process_owner import OrdinaryChild, register_process


def tmux_command(runner, socket, *arguments, check=True):
    """A short owned utility may exit before it becomes a signal owner.

    That is a normal terminal Popen result, never permission to adopt/signal a
    changed PID. Live qualification failures still stop the operation.
    """
    runner.utility_sequence = getattr(runner, "utility_sequence", 0) + 1
    receipt = runner.output / ("tmux-utility-%d-owner.jsonl" % runner.utility_sequence)
    process = subprocess.Popen(["/usr/bin/tmux", "-S", str(socket), *arguments], env=isolated_environment(),
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    child = OrdinaryChild(process, receipt)
    runner.nodes.append(child)
    primary, qualification = None, None
    try:
        try:
            child.owner = register_process(process, receipt)
        except RuntimeError as failure:
            qualification = str(failure)
            if process.poll() is None:
                raise
        output, errors = child.communicate(timeout=5)
        runner.record("controller", "tmux_utility", {"exit": process.returncode, "registered": child.owner is not None,
                      "qualification": qualification, "stdout_bytes": len(output), "stderr_bytes": len(errors)})
        result = subprocess.CompletedProcess(process.args, process.returncode, output, errors)
        if check:
            assert result.returncode == 0, errors
        return result
    except BaseException as failure:
        primary = failure
        raise
    finally:
        try:
            child.close()
        except BaseException as failure:
            if primary is None:
                raise
            primary.add_note("Independent tmux utility cleanup: " + str(failure))


class PrivateTmux:
    def __init__(self, runner):
        self.runner = runner
        self.socket = runner.private / "precondition.sock"
        self.config = runner.private / "precondition.conf"
        self.config.write_text("set -g exit-empty off\nset -g history-limit 128\n")
        self.logs, self.process, self.reader = [], None, self
        runner.targets.append(self)
        for name in ["stdout", "stderr"]:
            self.logs.append((runner.output / ("tmux-server." + name)).open("wb"))
        self.process = subprocess.Popen(["/usr/bin/tmux", "-D", "-S", str(self.socket), "-f", str(self.config)],
                                        env=isolated_environment(), stdin=subprocess.DEVNULL,
                                        stdout=self.logs[0], stderr=self.logs[1])
        # tmux changes its own argv. Preserve its original Popen/start identity;
        # never register the changed argv as permission to send an OS signal.
        self.original = identity(self.process.pid)
        runner.record("controller", "tmux_popen", {"pid": self.process.pid, "identity": self.original,
                      "argv": self.process.args, "os_signal_permitted": False})
        runner.until(lambda: self.socket.exists() or self.process.poll() is not None, "private tmux startup")
        assert self.process.poll() is None
        marker = runner.private / "precondition-marker"
        self.command("new-session", "-d", "-s", "owned-precondition", "/bin/sh", "-c",
                     "printf 'owned-pty-precondition' > " + str(marker) + "; exec /bin/cat")
        runner.until(marker.exists, "actual tmux command effect")
        assert marker.read_bytes() == b"owned-pty-precondition"
        tty = self.command("display-message", "-p", "-t", "owned-precondition", "#{pane_tty}").strip()
        assert tty.startswith("/dev/pts/"), tty
        runner.record("controller", "tmux_precondition", {"actual_command_effect": True, "actual_pty": True,
                      "fresh_socket": str(self.socket), "version": self.command("display-message", "-p", "#{version}").strip()})

    def command(self, *args):
        return tmux_command(self.runner, self.socket, *args).stdout

    def is_alive(self):
        return self.process is not None and self.process.poll() is None

    def close(self):
        failures = []
        if self.process is None:
            for stream in self.logs:
                stream.close()
            self.cleanup = {"owner":"private-tmux","started":False,"terminal":True,"exit":None,"errors":[],"os_signals":[]}
            return self.cleanup
        if self.process.poll() is None:
            try:
                self.command("kill-server")
            except BaseException as failure:
                failures.append(str(failure))
        try:
            self.runner.until(lambda: self.process.poll() is not None, "owned tmux server did not settle", seconds=5)
        except BaseException as failure:
            failures.append(str(failure))
        for stream in self.logs:
            stream.close()
        self.cleanup = {"owner": "private-tmux", "exit": self.process.poll(), "errors": failures,
                        "os_signals": [], "socket_removed": not self.socket.exists()}
        assert self.process.poll() == 0 and not failures, self.cleanup
        return self.cleanup


class WorkspaceObserver:
    """Only this run's SDK-created private tmux server, never an OS signal."""
    def __init__(self, runner, directory):
        self.runner, self.directory = runner, directory
        self.socket = directory / "server.sock"
        self.reader = self
        self.cleanup = None
        self.server = None
        runner.targets.append(self)

    def observe_server(self):
        pid = int(tmux_command(self.runner, self.socket, "display-message", "-p", "#{pid}").stdout.strip())
        self.server = identity(pid)
        self.runner.record("controller", "sdk_tmux_server_observed", {"identity": self.server, "os_signal_permitted": False})

    def is_alive(self):
        if self.server is not None:
            try:
                current = identity(self.server["pid"])
                stat = Path("/proc/%d/stat" % self.server["pid"]).read_text()
            except (FileNotFoundError, ProcessLookupError):
                return False
            if current["start_ticks"] != self.server["start_ticks"]:
                return False
            return stat[stat.rfind(")") + 2:].split()[0] != "Z"
        if not self.socket.exists():
            return False
        result = tmux_command(self.runner, self.socket, "list-sessions", check=False)
        if result.returncode == 0:
            return True
        assert "no server running on " in result.stderr or "No such file or directory" in result.stderr, result.stderr
        return False

    def close(self):
        if self.cleanup is not None:
            return self.cleanup
        failures, terminal = [], False
        try:
            if self.is_alive():
                tmux_command(self.runner, self.socket, "kill-server")
        except BaseException as failure:
            failures.append(str(failure))
        try:
            terminal = not self.is_alive()
        except BaseException as failure:
            failures.append(str(failure))
        self.cleanup = {"owner": "sdk-private-tmux", "terminal": terminal, "settlement_unknown": not terminal,
                        "errors": failures, "os_signals": []}
        assert terminal and not failures, self.cleanup
        return self.cleanup



class TerminalRunner(WebRunner):
    def run(self):
        started, primary = time.monotonic(), None
        report = {"status": "failed", "cleanup": [], "cleanup_errors": []}
        try:
            self.scenario()
            report["status"] = "pass"
        except BaseException as failure:
            primary = failure
            report.update(failure_type=type(failure).__name__, failure=str(failure))

        def clean_owners():
            for owner in list(reversed(self.nodes)) + list(reversed(self.targets)):
                try:
                    report["cleanup"].append(owner.close())
                except BaseException as failure:
                    report["cleanup_errors"].append({"owner":type(owner).__name__,"type":type(failure).__name__,"message":str(failure)})
                    report["cleanup"].append(getattr(owner, "cleanup", None))
            if report["cleanup_errors"]:
                report["status"] = "failed"

        def observe_terminal():
            unsettled = []
            for owner in list(self.nodes) + list(self.targets):
                try:
                    terminal = owner.terminal() if hasattr(owner, "terminal") else not owner.reader.is_alive()
                except BaseException as failure:
                    terminal = False
                    report["cleanup_errors"].append({"owner":type(owner).__name__,"step":"terminal-observation","type":type(failure).__name__,"message":str(failure)})
                if not terminal:
                    unsettled.append(type(owner).__name__)
            report["all_owned_terminal"] = not unsettled
            report["unsettled"] = unsettled
            if unsettled or report["cleanup_errors"]:
                report["status"] = "failed"

        def remove_private():
            if report.get("all_owned_terminal") and not report["cleanup_errors"]:
                assert (self.private / ".owner").read_text() == str(self.output)
                shutil.rmtree(self.private)
            report["private_state_removed"] = not self.private.exists()
            if self.private.exists():
                report["status"] = "failed"

        def provenance():
            end, drift = {}, []
            for path, digest in self.controller_sources.items():
                try:
                    end[path] = hashlib.sha256(Path(path).read_bytes()).hexdigest()
                    if end[path] != digest:
                        drift.append(path)
                except BaseException as failure:
                    drift.append(path)
                    report["cleanup_errors"].append({"step":"controller-provenance","type":type(failure).__name__,"message":str(failure)})
            report["controller_drift"] = drift
            if drift:
                report["status"] = "failed"
            (self.output / "controller-end.json").write_text(json.dumps(end, indent=2)+"\n")

        def write_result():
            report["seconds"] = time.monotonic() - started
            (self.output / "result.json").write_text(json.dumps(report, indent=2)+"\n")

        def checked(label, action):
            def execute():
                try:
                    return action()
                except BaseException as failure:
                    report["status"] = "failed"
                    report["cleanup_errors"].append({"step":label,"type":type(failure).__name__,"message":str(failure)})
                    raise
            return label, execute
        try:
            self.finish_scenario(primary, [checked("owners", clean_owners), checked("terminal", observe_terminal),
                checked("private", remove_private), checked("provenance", provenance), checked("result", write_result)])
        except BaseException as failure:
            report["status"] = "failed"
            try:
                write_result()
            except BaseException as secondary:
                failure.add_note("Final terminal result persistence failed: " + str(secondary))
            raise
        assert report["status"] == "pass", report

    def record(self, node, operation, result):
        if operation in {"terminal_read", "read_terminal"} and result.get("ok"):
            response = dict(result["result"])
            if response.get("message", {}).get("type") == "bytes":
                data = bytes(response["message"]["data"])
                response["message"] = {"type": "bytes", "length": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            result = dict(result, result=response)
        super().record(node, operation, result)

    def send(self, node, attachment, text):
        node.call("terminal_send", attachment=attachment, message={"type": "bytes", "data": list(text.encode())})

    def read_until(self, node, attachment, marker):
        data = bytearray()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            response = node.call("terminal_read", attachment=attachment, timeout_ms=100)
            if response.get("message", {}).get("type") == "bytes":
                data.extend(response["message"]["data"])
                assert len(data) <= 128 * 1024, "terminal observation exceeded bounded expected output"
                if marker in data:
                    return bytes(data)
            assert response.get("attached", False), response
        raise AssertionError("expected exact terminal output marker not observed")

    def workspaces(self, provider, consumer, directory):
        observer = WorkspaceObserver(self, directory)
        first = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        second = consumer.call("terminal_create", address=provider.address)["workspace"]
        assert first != second
        listed = consumer.call("terminal_list", address=provider.address)["workspaces"]
        assert set(listed) == {first, second}
        attachments = [consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"] for workspace in [first, second]]
        for index, (workspace, attachment) in enumerate(zip([first, second], attachments)):
            self.send(consumer, attachment, "stty -echo; printf 'READY_'; printf 'M120\\n'\n")
            self.read_until(consumer, attachment, b"READY_M120")
            marker = "OWNED_M120_%d" % index
            self.send(consumer, attachment, "printf '%s\\n' " + marker + "; printf 'workspace-%d' > observed; test -t 0 && printf 'REAL_PTY_%d\\n'\n" % (index, index))
            self.read_until(consumer, attachment, ("REAL_PTY_%d" % index).encode())
            assert (directory / workspace / "observed").read_bytes() == ("workspace-%d" % index).encode()
            tty = tmux_command(self, observer.socket, "display-message", "-p", "-t", workspace + ":0.0", "#{pane_tty}").stdout.strip()
            assert tty.startswith("/dev/pts/"), "owned tmux pane has no PTY: " + repr(tty)
        self.record("controller", "real_concurrent_workspaces", {"count": 2, "independent_file_effects": True, "pty": True})
        # The owned job remains live while attachment is explicitly detached.
        self.send(consumer, attachments[0], "(sleep 1; printf 'retained-work' > retained) &\n")
        started = time.monotonic()
        detached = consumer.call("terminal_detach", attachment=attachments[0])
        elapsed = time.monotonic() - started
        assert elapsed <= 6.25, "public detach exceeded original protocol plus cleanup observation"
        self.record("controller", "public_detach_elapsed", {"seconds":elapsed,"bound":6.25})
        assert detached == {"attached": False, "workspace_ended": False, "failed": False}, detached
        self.until((directory / first / "retained").exists, "detached job did not continue")
        assert (directory / first / "retained").read_bytes() == b"retained-work"
        assert set(consumer.call("terminal_list", address=provider.address)["workspaces"]) == {first, second}
        reattached = consumer.call("terminal_attach", address=provider.address, workspace=first)["id"]
        self.send(consumer, reattached, "printf 'SAME_WORKSPACE\\n'; cat retained\n")
        self.read_until(consumer, reattached, b"retained-work")
        self.record("controller", "authorized_reattach", {"same_workspace": True, "retained_job_effect": True})
        ended = consumer.call("terminal_end", address=provider.address, workspace=first)
        assert ended["ended"] is True and ended["workspace"] == first
        assert consumer.call("terminal_list", address=provider.address)["workspaces"] == [second]
        consumer.call("terminal_cancel", attachment=reattached)
        self.send(consumer, attachments[1], "printf 'OTHER_WORKSPACE_ALIVE\\n'\n")
        self.read_until(consumer, attachments[1], b"OTHER_WORKSPACE_ALIVE")
        consumer.call("terminal_detach", attachment=attachments[1])
        consumer.call("terminal_end", address=provider.address, workspace=second)
        assert consumer.call("terminal_list", address=provider.address)["workspaces"] == []
        self.record("controller", "selected_explicit_end", {"selected_ended": True, "other_independent": True, "all_explicitly_ended": True})

    def input_boundary(self, provider, consumer, directory):
        observer = WorkspaceObserver(self, directory)
        workspace = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        attachment = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
        self.send(consumer, attachment, "stty -echo; printf 'INPUT_'; printf 'READY\\n'\n")
        self.read_until(consumer, attachment, b"INPUT_READY")
        self.send(consumer, attachment, "stty -icanon min 1 time 0; printf 'started' > input-start; head -c 16384 > input.bin; printf 'INPUT_'; printf 'COMPLETE\\n'\n")
        self.until((directory/workspace/"input-start").exists, "owned real input reader did not start")
        primary = None
        try:
            consumer.call("terminal_send", attachment=attachment, message={"type":"bytes","data":[81]*16384})
            self.read_until(consumer, attachment, b"INPUT_COMPLETE")
            assert (directory/workspace/"input.bin").read_bytes() == b"Q"*16384
        except BaseException as failure:
            primary = failure
        actions = [
            ("provider-status", lambda: self.record("controller", "input_boundary_provider", provider.call("terminal_status"))),
            ("consumer-status", lambda: self.record("controller", "input_boundary_consumer", consumer.call("terminal_status"))),
            ("owned-file", lambda: self.record("controller", "input_boundary_independent_file", {"bytes":(directory/workspace/"input.bin").stat().st_size,
                     "sha256":hashlib.sha256((directory/workspace/"input.bin").read_bytes()).hexdigest()})),
        ]
        self.finish_scenario(primary, actions)
        assert not consumer.call("terminal_detach", attachment=attachment)["failed"]
        consumer.call("terminal_end", address=provider.address, workspace=workspace)
        self.record("controller", "actual_max_input_record", {"bytes":16384,"actual_reader_exact":True})

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        PrivateTmux(self)
        provider = OwnedNode(self, "provider")
        consumer = self.member(provider, "consumer")
        assert provider.identity != consumer.identity
        assert consumer.call("ping", address=provider.address)["reply"] == "PONG"
        self.record("controller", "public_member_precondition", {"distinct_identities": True, "actual_reply": True})
        directory = self.private / "terminal-provider"
        enabled = provider.call("terminal_enable", directory=str(directory))
        assert enabled["capable"] and enabled["enabled"] and enabled["active_operations"] == 0
        if self.case == "workspaces":
            self.workspaces(provider, consumer, directory)
        elif self.case == "input-bound":
            self.input_boundary(provider, consumer, directory)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="entry")
    TerminalRunner(parser.parse_args()).run()
