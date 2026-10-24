#!/usr/bin/env python3
"""Run a verified public SDK lifecycle consumer without invoking Cargo."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
from types import SimpleNamespace
from check_forwarding import isolated_environment
from forwarding_local_resources import OwnedProcess, LoopbackRelay


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--test-filter")
    parser.add_argument("--relay-binary", type=Path)
    parser.add_argument("--relay-receipt", type=Path)
    args = parser.parse_args()
    build = json.loads(args.build_receipt.read_text())
    binary = Path(build["pin"])
    assert build["exit"] == 0 and build["CargoInvoked"] is False and build["dependencyRebuilds"] == 0
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["sha256"]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    private = Path(tempfile.mkdtemp(prefix="m118-forward-sdk-"))
    (private / ".owner").write_text(str(output))
    environment = isolated_environment()
    environment["M118_FORWARD_PRIVATE"] = str(private)
    started = time.monotonic()
    report = {"status": "failed", "pin_sha256": build["sha256"], "consumer_receipt": str(args.build_receipt),
              "SDK_receipt": build["SDKReceipt"], "test_sha256": build["testSHA256"]}
    process = None
    owner = None
    relay = None
    cleanup_errors = []
    try:
        if args.relay_binary or args.relay_receipt:
            assert args.relay_binary and args.relay_receipt
            assert args.test_filter in {"configured_guest_resources_preserve_local_allow_list_after_reopen", "actual_pending_forward_can_cancel_drop_and_reopen_without_late_effect", "provider_drop_at_capacity_closes_both_sockets_without_late_effect"}, "local-only resource consumer must select an explicit local-resource test"
            sources = {}
            def freeze(path):
                sources[str(path)] = hashlib.sha256(Path(path).read_bytes()).hexdigest()
                (output / "controller-start.json").write_text(json.dumps(sources, indent=2) + "\n")
            def record(label, operation, value):
                with (output / "events.jsonl").open("a") as log:
                    log.write(json.dumps(dict(owner=label, operation=operation, result=value)) + "\n")
            fixture = SimpleNamespace(output=output, nodes=[], freeze_source=freeze, record=record)
            relay = LoopbackRelay(fixture, args.relay_binary, args.relay_receipt)
            environment["M118_LOCAL_RELAY"] = relay.url
        command = [str(binary), "--nocapture", "--test-threads=1"] + ([args.test_filter] if args.test_filter else [])
        with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open("w") as stderr:
            process = subprocess.Popen(command, env=environment, stdout=stdout, stderr=stderr)
            owner = OwnedProcess(process, output / "owner.jsonl")
            report["exit"] = process.wait(timeout=120)
        assert report["exit"] == 0, "public SDK lifecycle consumer failed; original logs/private state retained"
        report["status"] = "pass"
    except BaseException as failure:
        report.update(failure_type=type(failure).__name__, failure=str(failure))
        raise
    finally:
        # A failed or unproved owner retains its private evidence. The normal
        # path has no signals; these are only this Popen's own timeout cleanup.
        if process is not None and process.poll() is None:
            try:
                if owner is None:
                    raise RuntimeError("live child ownership was not registered; no signal sent")
                owner.send(signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    owner.send(signal.SIGKILL)
                    process.wait(timeout=5)
            except BaseException as failure:
                cleanup_errors.append(type(failure).__name__)
        if owner is not None and process.poll() is not None:
            try:
                owner.close()
            except BaseException as failure:
                cleanup_errors.append(type(failure).__name__)
        if relay is not None:
            try:
                report["relay_cleanup"] = relay.close()
            except BaseException as failure:
                cleanup_errors.append(type(failure).__name__)
        terminal = (process is None or process.poll() is not None) and (relay is None or relay.terminal())
        if report["status"] == "pass" and terminal and not cleanup_errors:
            try:
                assert (private / ".owner").read_text() == str(output)
                shutil.rmtree(private)
            except BaseException as failure:
                cleanup_errors.append(type(failure).__name__)
        if cleanup_errors or not terminal:
            report["status"] = "failed"
        report.update(seconds=time.monotonic() - started, all_owned_terminal=terminal, cleanup_errors=cleanup_errors,
                      private_state_removed=not private.exists(), exit=process.poll() if process else None)
        if private.exists():
            report["retained_private_directory"] = str(private)
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    assert report["status"] == "pass", report


if __name__ == "__main__":
    main()
