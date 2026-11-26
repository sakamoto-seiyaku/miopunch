#!/usr/bin/env python3
"""Qualify NativeClient migration through independent native CLI processes."""
import argparse
import json
from pathlib import Path
import time
from types import SimpleNamespace

from check_native_client import Host as NativeHost, Runner
from forwarding_local_resources import LoopbackRelay


def wait_for(read, accepts, timeout=24):
    deadline = time.monotonic() + timeout
    while True:
        value = read()
        if accepts(value):
            return value
        assert time.monotonic() < deadline, value
        time.sleep(0.15)


class Host(NativeHost):
    def read(self):
        with (self.runner.output / (self.label + ".stdout")).open("w") as raw:
            for line in self.process.stdout:
                raw.write(line)
                if line.strip():
                    self.lines.put(line)
        self.lines.put(None)


class MigrationRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.nodes = []
        self.relay = LoopbackRelay(self, args.relay_binary, args.relay_receipt)
        self.configuration = self.write_configuration("resources.json", self.relay.url)
        self.args = args

    def write_configuration(self, name, relay, replace=True):
        path = self.output / name
        resources = [dict(url=relay, uses=["DataRelay"], limitation="owned loopback fixture")]
        path.write_text(json.dumps(dict(
            defaults=dict(revision=2, resources=[] if replace else resources),
            replacement=resources if replace else None,
            disabled=[], disabled_uses=["Observation", "Lookup", "Signaling"],
            lan_discovery="Disabled", port_mapping="Disabled",
            udp_direct_disabled=True, tcp_direct_disabled=True,
            maintenance=dict(contact_timeout_ms=7000, exploration_interval_ms=500),
        )))
        return path

    def freeze_source(self, _path):
        pass

    def native_configuration(self, _remote):
        return str(self.configuration)

    def start(self, label, remote, action="create", invitation=None):
        return Host(self, label, remote, action, self.path(label + "-state", remote), invitation)

    def create_ready(self, name, configuration=None):
        host = Host(self, name, False, "create", self.path(name + "-state", False),
                    configuration=str(configuration) if configuration else None)
        host.call("export_recovery_card", path=self.path(name + "-recovery-card.json", False))
        host.call("acknowledge_recovery_saved")
        return host

    def open_again(self, host, stop=True):
        if stop:
            host.stop()
        opened = Host(self, host.label + "-reopen", False, "open", host.directory)
        assert opened.node == host.node
        return opened

    def invite(self, provider, label, use_precode=False):
        path = self.path(label + ("-precode.json" if use_precode else "-invite.json"), False)
        if use_precode:
            provider.call("issue_precode", path=path, seconds=600, max_uses=1)
        else:
            provider.call("invite", path=path)
        return path

    def admit(self, provider, label):
        invitation = self.invite(provider, label)
        member = self.start(label, False, "join", invitation)
        assert member.call("request", path=invitation)["stage"] == "AwaitingApproval"
        requests = provider.call("requests")["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == member.node
        provider.call("approve", **requests[0])
        assert member.call("request", path=invitation)["stage"] == "Complete"
        return member

    def address_of(self, node, relay=None):
        return dict(id=node.node, addrs=[dict(Relay=relay or self.relay.url)])

    def pong(self, a, b, relay=None):
        relay = relay or self.relay.url
        result = a.call("ping", address=self.address_of(b, relay))
        assert result["reply"] == "PONG"
        assert [p["remote"] for p in result["transport"]["paths"] if p["selected"]] == [dict(Relay=relay)]

    @staticmethod
    def progress_on(host, network, matches):
        snapshot = wait_for(lambda: host.call("state_snapshot"), lambda s: matches(s["network_migration"]))
        assert snapshot["status"]["network"] == network, snapshot
        assert snapshot["network_migration"] == host.call("migration_progress")
        return snapshot["network_migration"]

    @staticmethod
    def role(host, node):
        return next((member["role"] for member in host.call("status")["members"] if member["node"] == node), None)

    def lost_genesis(self):
        mover = self.create_ready("lost-mover")
        witness = self.admit(mover, "old-witness")
        destination = self.create_ready("new-admin")
        old_network = mover.ready["status"]["network"]
        new_network = destination.ready["status"]["network"]
        assert old_network != new_network and mover.node != destination.node
        invitation = self.invite(destination, "migration")
        assert mover.call("migration_progress") is None
        assert destination.call("requests")["requests"] == []
        mover.stop()
        # This owned fixture destroys both its stored credential and its only exported card.
        (Path(mover.directory) / "genesis.credential").unlink()
        Path(self.path("lost-mover-recovery-card.json", False)).unlink()
        mover = self.open_again(mover, stop=False)
        assert not mover.call("device_operations")["holds_genesis"]
        self.pong(witness, mover)

        before = mover.call("state_snapshot")["cursor"]
        rejected = self.invite(mover, "same-network")
        mover.call("begin_network_migration", ok=False, invitation_path=rejected)
        assert mover.call("migration_progress") is None
        mover.call("begin_network_migration", invitation_path=invitation)
        assert mover.call("observe_state", after=before)["network_migration"] is not None
        waiting = self.progress_on(mover, old_network, lambda p: p["admission"] and p["admission"]["progress"] == "Delivered")
        assert not waiting["switched"] and waiting["source_exit"]["stage"] == "NotStarted"
        requests = destination.call("requests")["requests"]
        assert len(requests) == 1 and requests[0]["applicant"] == mover.node
        assert requests[0]["request_id"] == waiting["admission"]["request_id"]
        self.pong(witness, mover)
        assert self.role(destination, mover.node) is None
        mover.call("ping", ok=False, address=self.address_of(destination))

        destination.stop()
        mover.stop()
        anchor = Path(mover.directory) / "migration-target" / "network.anchor"
        unavailable = anchor.with_suffix(".unavailable")
        anchor.rename(unavailable)
        mover = self.open_again(mover, stop=False)
        self.progress_on(mover, old_network, lambda p: p and not p["switched"])
        self.pong(witness, mover)
        unavailable.rename(anchor)
        self.progress_on(mover, old_network, lambda p: p == waiting)
        self.pong(mover, witness)
        self.pong(witness, mover)
        mover.call("interrupt_migration_after_authorization")
        destination = self.open_again(destination, stop=False)
        assert destination.call("requests")["requests"] == requests
        destination.call("approve", **requests[0])
        assert mover.process.wait(timeout=15) == 86
        mover.stderr.close()
        self.hosts.remove(mover)
        destination.call("migration_control_only", enabled=True)
        destination.stop()
        mover = self.open_again(mover, stop=False)
        pending = self.progress_on(mover, old_network, lambda p: p["phase"] == "PendingMaterials")
        assert pending["admission"]["request_id"] == waiting["admission"]["request_id"]
        assert pending["admission"]["progress"] == "Authorized"
        witness = self.open_again(witness)
        self.pong(mover, witness)
        self.pong(witness, mover)
        destination = self.open_again(destination, stop=False)
        destination.call("migration_control_only", enabled=True)
        saved = self.progress_on(mover, old_network, lambda p: p["phase"] == "MaterialsSaved")
        assert saved["admission"]["progress"] == "Complete" and not saved["switched"]
        assert self.role(destination, mover.node) == "member"
        time.sleep(1)
        self.pong(witness, mover)
        self.pong(mover, witness)
        mover = self.open_again(mover)
        self.progress_on(mover, old_network, lambda p: p == saved)
        self.pong(witness, mover)

        # Force a target runtime preflight failure only after the destination
        # can prove a member data path. The old process must still serve.
        resources = Path(mover.directory) / "migration-target" / "resources.json"
        valid_resources = resources.read_text()
        invalid = json.loads(valid_resources)
        invalid["maintenance"]["contact_timeout_ms"] = 0
        resources.write_text(json.dumps(invalid))
        destination.call("migration_control_only", enabled=False)
        checked = self.progress_on(mover, old_network, lambda p: p["phase"] == "Validated")
        assert not checked["switched"]
        self.pong(witness, mover)
        resources.write_text(valid_resources)
        switched = self.progress_on(mover, new_network, lambda p: p["switched"])
        assert switched["source_exit"] == dict(stage="GenesisConfirmationRequired")
        assert switched["admission"]["request_id"] == waiting["admission"]["request_id"]
        self.pong(mover, destination)
        self.pong(destination, mover)
        witness.call("ping", ok=False, address=self.address_of(mover))
        mover.call("confirm_migration_source_exit", ok=False)
        mover = self.open_again(mover)
        self.progress_on(mover, new_network, lambda p: p == switched)
        witness = self.open_again(witness)
        assert self.role(witness, mover.node) == "admin"
        self.pong(destination, mover)
        self.record("runner", "lost_genesis_qualified", switched)

    def ordinary_departure(self):
        owner = self.create_ready("ordinary-owner")
        mover = self.admit(owner, "ordinary-mover")
        target = self.create_ready("ordinary-target")
        destination_network = target.ready["status"]["network"]
        invitation = self.invite(target, "ordinary-migration", use_precode=True)
        target.call("migration_control_only", enabled=True)
        mover.call("begin_network_migration", invitation_path=invitation)
        saved = self.progress_on(mover, owner.ready["status"]["network"], lambda p: p["phase"] == "MaterialsSaved")
        self.pong(owner, mover)
        owner.stop()
        target.call("migration_control_only", enabled=False)
        pending = self.progress_on(mover, destination_network, lambda p: p["source_exit"]["stage"] == "PendingDelivery")
        assert pending["switched"] and pending["admission"] == saved["admission"]
        self.pong(target, mover)
        mover = self.open_again(mover)
        self.progress_on(mover, destination_network, lambda p: p == pending)
        owner = self.open_again(owner, stop=False)
        done = self.progress_on(mover, destination_network, lambda p: p["source_exit"]["stage"] == "Delivered")
        assert done["source_exit"]["record"] == pending["source_exit"]["record"]
        assert done["source_exit"]["recipient"] == owner.node
        owner = self.open_again(owner)
        assert self.role(owner, mover.node) is None
        mover = self.open_again(mover)
        self.progress_on(mover, destination_network, lambda p: p == done)
        assert target.call("precode_status", path=invitation)["uses"] == 1
        self.pong(mover, target)
        self.record("runner", "ordinary_departure_qualified", done)

    def destination_only_relay(self):
        directory = self.output / "destination-relay"
        directory.mkdir()
        relay = LoopbackRelay(SimpleNamespace(
            output=directory, nodes=[], freeze_source=self.freeze_source, record=self.record,
        ), self.args.relay_binary, self.args.relay_receipt)
        try:
            self.configuration = self.write_configuration("source-defaults.json", self.relay.url, replace=False)
            new_config = self.write_configuration("destination-resources.json", relay.url)
            source = self.create_ready("separate-source")
            mover = self.admit(source, "separate-mover")
            target = self.create_ready("separate-target", new_config)
            invite = self.invite(target, "separate-migration")
            mover.call("begin_network_migration", invitation_path=invite)
            waiting = self.progress_on(mover, source.ready["status"]["network"],
                                       lambda p: p["admission"] and p["admission"]["progress"] == "Delivered")
            requests = target.call("requests")["requests"]
            assert len(requests) == 1 and requests[0]["request_id"] == waiting["admission"]["request_id"]
            self.pong(source, mover)
            self.pong(mover, source)
            target.call("approve", **requests[0])
            done = self.progress_on(mover, target.ready["status"]["network"],
                                    lambda p: p["source_exit"]["stage"] == "Delivered")
            self.pong(target, mover, relay.url)
            mover = self.open_again(mover)
            self.progress_on(mover, target.ready["status"]["network"], lambda p: p == done)
            self.pong(mover, target, relay.url)
            self.record("runner", "destination_only_relay_qualified", done)
        finally:
            self.record("runner", "destination_relay_cleanup", relay.close())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--linux", required=True, type=Path)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--relay-binary", required=True, type=Path)
    parser.add_argument("--relay-receipt", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--scenario", choices=("all", "lost-genesis", "ordinary", "destination-relay"), default="all")
    args = parser.parse_args()
    args.windows, args.windows_sha, args.local_only, args.failure_only = None, None, True, False
    runner = MigrationRunner(args)
    passed = False
    try:
        if args.scenario in ("all", "lost-genesis"):
            runner.lost_genesis()
        if args.scenario in ("all", "ordinary"):
            runner.ordinary_departure()
        if args.scenario in ("all", "destination-relay"):
            runner.destination_only_relay()
        passed = True
    finally:
        cleanup = runner.cleanup()
        relay_cleanup = runner.relay.close()
        passed = passed and all("error" not in result for result in cleanup)
        (runner.output / "result.json").write_text(json.dumps(dict(
            passed=passed, host_cleanup=cleanup, relay_cleanup=relay_cleanup,
        ), indent=2) + "\n")
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
