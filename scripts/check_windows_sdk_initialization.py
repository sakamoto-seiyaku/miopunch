#!/usr/bin/env python3
"""One ordinary public create, join and open on the frozen diagnostic SDK11."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import time

from web_windows_process import WindowsChild

SDK_SHA = "70aba7ef847b4ca4409c28d643ba6ac1a5bf32ecbc28b6ed48c70529cd1c1f40"
SDK_NAME = "sdk/miopunch-client-sdk11-init-diagnostic.exe"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", required=True)
    args = parser.parse_args()
    assert os.name == "nt" and sys.maxsize > 2**32 and sys.flags.isolated and sys.flags.no_site
    root, output = args.root.resolve(), args.output.resolve()
    assert output.is_relative_to(root)
    output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((root / args.manifest).read_text())
    assert all(sha(root / name) == digest for name, digest in manifest["files"].items())
    binary = root / SDK_NAME
    assert sha(binary) == SDK_SHA
    private = output / "private"
    private.mkdir()
    for name in ["temp", "local-appdata", "appdata"]:
        (private / name).mkdir()
    windows = Path(os.environ["SystemRoot"])
    environment = dict(SystemRoot=str(windows), WINDIR=str(windows), PATH=str(windows / "System32"),
                       TEMP=str(private / "temp"), TMP=str(private / "temp"),
                       LOCALAPPDATA=str(private / "local-appdata"), APPDATA=str(private / "appdata"))
    configuration = private / "resources.json"
    configuration.write_text(json.dumps(dict(defaults=dict(revision=2, resources=[]), replacement=[], disabled=[],
        disabled_uses=["Observation", "Lookup", "Signaling"], lan_discovery="Disabled", port_mapping="Disabled")))
    nodes, records, cleanup, errors = [], [], [], []
    result = dict(result="FAILED", scope="startup localization only; no Web/business acceptance or product fix",
                  SDK_SHA256=SDK_SHA, whole_host_zero_egress=False,
                  intended_public_entries=["create", "join", "open"], first_error_stops_remaining_entries=True,
                  join_scope="pre-admission startup readiness; no request/redeem or JoinComplete claim")
    primary = None

    def record(operation, value):
        row = dict(operation=operation, value=value, monotonic=time.monotonic())
        records.append(row)
        with (output / "events.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(row) + "\n")

    def start(label, action, directory, invitation=None):
        owner = dict(label=label, action=action, child=None, child_registry=[], ready=False, retired=False)
        nodes.append(owner)
        argv = [str(binary), action, str(directory)]
        if invitation is not None:
            argv.append(str(invitation))
        if action != "open":
            argv.append(str(configuration))
        record("public_entry_intent", dict(label=label, action=action, fresh_process=True,
                                          resource_config_argument=action != "open"))
        child = WindowsChild(argv, environment, output, label, response_timeout=80,
                             owners=owner["child_registry"])
        owner["child"] = child
        ready = child.wait_ready(timeout=80)
        owner["ready"] = True
        owner["status"] = ready["status"]
        record("public_entry_ready", dict(label=label, action=action, stage=ready["status"]["stage"],
            original_pid=child.process.pid, original_handle=child.handle))
        return owner

    def stop(owner):
        child = owner["child"]
        assert child.call("stop") == dict(stopped=True)
        settled = child.join(timeout=20)
        owner["retired"] = True
        cleanup.append(settled)
        record("public_entry_normal_stop", settled)

    try:
        creator_directory = private / "creator"
        joiner_directory = private / "joiner"
        assert not creator_directory.exists() and not joiner_directory.exists()
        creator = start("creator", "create", creator_directory)
        assert creator["status"]["stage"] == "AwaitingRecoverySave"
        # The fresh synthetic card stays at this endpoint; no card contents
        # are read, hashed, logged or transferred. This is the M113 baseline.
        child = creator["child"]
        assert child.call("export_recovery_card", path=str(private / "creator-synthetic-card")) == dict(saved=True)
        assert child.call("acknowledge_recovery_saved") == dict(saved_acknowledged=True)
        invitation = private / "joiner-invitation"
        assert child.call("issue_precode", path=str(invitation), seconds=600, max_uses=1) == dict(saved=True)
        record("fresh_baseline_invitation_saved_locally", dict(private_contents_not_read_or_transferred=True))
        creator_identity = creator["status"]["node"]
        joiner = start("joiner", "join", joiner_directory, invitation)
        assert joiner["status"]["stage"] == "AwaitingApproval" and joiner["status"]["node"] != creator_identity
        record("independent_join_identity", dict(independent=True, stage="AwaitingApproval"))
        joiner_identity = joiner["status"]["node"]
        stop(joiner)
        # Open the SAME newly owned synthetic joiner only after its original
        # process and readers joined. Open has no resource-file CLI argument.
        reopened = start("opened-joiner", "open", joiner_directory)
        assert reopened["status"]["node"] == joiner_identity
        assert reopened["status"]["stage"] == "AwaitingApproval"
        record("same_newly_owned_identity_after_normal_stop_open", dict(same_identity=True))
        stop(reopened)
        stop(creator)
        result["result"] = "UNLOCALIZED_WITHIN_THIS_CASE"
        result["all_three_public_entries_ready_and_normally_stopped"] = True
        result["original_failures_resolved"] = False
        record("all_selected_entries_succeeded_without_localizing_original_Os5", dict(
            no_retry=True, no_fix_claim=True, instrumentation_may_affect_timing=True))
    except BaseException as failure:
        primary = failure
        result["primary_failure"] = dict(type=type(failure).__name__, message=str(failure))
    finally:
        for owner in reversed(nodes):
            if owner["retired"]:
                continue
            child = owner["child"]
            if child is None and owner["child_registry"]:
                child = owner["child_registry"][0]
            if child is None:
                cleanup.append(dict(owner=owner["label"], process_created=False))
                continue
            try:
                if owner["ready"] and child.process.poll() is None:
                    child.call("stop")
                cleanup.append(child.join(timeout=20))
            except BaseException as failure:
                errors.append(dict(owner=owner["label"], type=type(failure).__name__,
                                   message=str(failure), actual_settlement=child.closed))
                if child.process.poll() is None or any(reader.is_alive() for reader in child.readers):
                    try:
                        cleanup.append(child.cleanup())
                    except BaseException as cleanup_failure:
                        errors.append(dict(owner=owner["label"], type=type(cleanup_failure).__name__,
                                           message=str(cleanup_failure)))
        unsettled = [owner["label"] for owner in nodes for child in owner["child_registry"]
                     if child.process.poll() is None or any(reader.is_alive() for reader in child.readers)]
        result.update(cleanup=cleanup, cleanup_errors=errors, unsettled=unsettled, events=len(records),
                      attempted_entries=[owner["label"] for owner in nodes],
                      created_entries=[owner["label"] for owner in nodes if owner["child_registry"]],
                      not_reached_entries=[label for label in ["creator", "joiner", "opened-joiner"]
                                           if label not in [owner["label"] for owner in nodes]],
                      source_drift=[name for name, digest in manifest["files"].items() if sha(root / name) != digest])
        if errors or unsettled or result["source_drift"]:
            result["result"] = "FAILED"
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result), flush=True)
    if primary:
        raise primary
    if result["result"] == "FAILED":
        raise RuntimeError("diagnostic startup ownership or source qualification failed")


if __name__ == "__main__":
    main()
