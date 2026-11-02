#!/usr/bin/env python3
"""Windows Native consumer against real Linux PTYs; no WSL provider operation."""
import argparse
import json
from pathlib import Path
import sys
import time

from check_forwarding_platform import WindowsNode
from check_terminal_service import TerminalRunner, WorkspaceObserver
from check_web_service import OwnedNode


class PlatformTerminalRunner(TerminalRunner):
    def __init__(self, args):
        super().__init__(args)
        self.windows_sdk, self.windows_sha = args.windows_sdk, args.windows_sha
        self.windows_terminal_consumer = True
        scripts = Path(__file__).resolve().parent
        for module in list(sys.modules.values()):
            path = getattr(module, "__file__", None)
            if path and Path(path).resolve().parent == scripts and Path(path).suffix == ".py":
                self.freeze_source(path)
        self.native_configuration = self.private / "resources.json"
        self.native_configuration.write_text(json.dumps({"defaults": {"revision": 2, "resources": []},
            "replacement": [], "disabled": [], "disabled_uses": ["Observation", "Lookup", "Signaling", "DataRelay"],
            "lan_discovery": "Disabled", "port_mapping": "Disabled"}))

    def scenario(self):
        provider = OwnedNode(self, "linux-provider")
        invitation = self.private / "windows-invitation.json"
        provider.call("issue_precode", path=str(invitation), seconds=1800, max_uses=1)
        consumer = WindowsNode(self, invitation)
        assert consumer.identity != provider.identity
        provider.call("service_sync", address=consumer.address)
        directory = self.private / "linux-terminal"
        consumer.call("terminal_create", address=provider.address, ok=False)
        assert not directory.exists()
        time.sleep(2.05)
        enabled = provider.call("terminal_enable", directory=str(directory))
        assert enabled["enabled"] and enabled["capable"]
        observer = WorkspaceObserver(self, directory)
        first = consumer.call("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        second = consumer.call("terminal_create", address=provider.address)["workspace"]
        assert first != second
        assert set(consumer.call("terminal_list", address=provider.address)["workspaces"]) == {first, second}
        attachments = []
        for index, workspace in enumerate([first, second]):
            identifier = consumer.call("terminal_attach", address=provider.address, workspace=workspace)["id"]
            attachments.append(identifier)
            self.send(consumer, identifier, "stty -echo; printf 'READY_'; printf 'WIN\\n'\n")
            self.read_until(consumer, identifier, b"READY_WIN")
            self.send(consumer, identifier, "printf 'workspace-%d' > observed; test -t 0 && printf 'REAL_'; printf 'PTY\\n'\n" % index)
            self.read_until(consumer, identifier, b"REAL_PTY")
            assert (directory / workspace / "observed").read_bytes() == ("workspace-%d" % index).encode()
        self.record("controller", "windows_consumer_two_real_ptys", {"workspaces": 2, "independent_file_effects": True})
        self.send(consumer, attachments[0], "(printf 'started' > started; while [ ! -f release ]; do sleep 0.05; done; printf 'retained-work' > retained) &\n")
        self.until((directory / first / "started").exists, "owned retained job start")
        assert not (directory / first / "retained").exists()
        started = time.monotonic()
        detached = consumer.call("terminal_detach", attachment=attachments[0])
        elapsed = time.monotonic() - started
        assert detached == {"attached": False, "workspace_ended": False, "failed": False}
        assert elapsed < 6.25, "detach exceeded original public bound including control latency"
        assert not (directory / first / "retained").exists()
        (directory / first / "release").write_text("release-owned-job")
        self.until((directory / first / "retained").exists, "detached retained job effect")
        assert (directory / first / "retained").read_bytes() == b"retained-work"
        reattached = consumer.call("terminal_attach", address=provider.address, workspace=first)["id"]
        self.send(consumer, reattached, "cat retained; printf '\\n'\n")
        self.read_until(consumer, reattached, b"retained-work")
        assert consumer.call("terminal_end", address=provider.address, workspace=first)["ended"]
        consumer.call("terminal_cancel", attachment=reattached)
        self.send(consumer, attachments[1], "printf 'OTHER_'; printf 'ALIVE\\n'\n")
        self.read_until(consumer, attachments[1], b"OTHER_ALIVE")
        assert consumer.call("terminal_list", address=provider.address)["workspaces"] == [second]
        provider.call("terminal_disable")
        self.until(lambda: provider.call("terminal_status")["attachment_process_owners"] == 0,
                   "disable retained an attachment owner", seconds=10)
        outcome = consumer.call("terminal_read", attachment=attachments[1], timeout_ms=100)
        assert not outcome["attached"] and outcome.get("failed"), outcome
        consumer.call("terminal_send", attachment=attachments[1],
                      message={"type": "bytes", "data": list(b"printf forbidden > forbidden\n")}, ok=False)
        assert not (directory / second / "forbidden").exists()
        consumer.call("terminal_cancel", attachment=attachments[1])
        before = sorted(p.name for p in directory.glob("mio-*"))
        consumer.call("terminal_create", address=provider.address, ok=False)
        assert sorted(p.name for p in directory.glob("mio-*")) == before
        time.sleep(2.05)
        provider.call("terminal_enable", directory=str(directory))
        assert consumer.call("terminal_list", address=provider.address)["workspaces"] == [second]
        assert consumer.call("terminal_end", address=provider.address, workspace=second)["ended"]
        assert consumer.call("terminal_list", address=provider.address)["workspaces"] == []
        self.record("controller", "windows_native_terminal_lifecycle", {"controlled_job_released_only_after_detach": True,
                    "detach_seconds": elapsed, "same_workspace_reattached": True, "selected_end_other_workspace_alive": True,
                    "disable_closes_access_without_ending_workspace": True, "denied_input_no_file_effect": True,
                    "all_workspaces_publicly_ended": True, "wsl_operations": 0})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--windows-sdk", required=True)
    parser.add_argument("--windows-sha", required=True)
    parser.set_defaults(case="windows-consumer")
    PlatformTerminalRunner(parser.parse_args()).run()
