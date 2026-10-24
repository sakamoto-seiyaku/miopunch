#!/usr/bin/env python3
"""Run the pinned public Web SDK consumer with fresh owned local resources."""
import argparse
import hashlib
import json
from pathlib import Path

from check_forwarding import isolated_environment
from check_web_service import WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process, run_process


class SDKRunner(WebRunner):
    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        build = json.loads(self.args.consumer_receipt.read_text())
        binary = Path(build["pin"])
        assert build["exit"] == 0 and build["CargoInvoked"] is False and build["dependencyRebuilds"] == 0
        assert Path(build["SDKReceipt"]) == self.args.source_receipt.resolve()
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["sha256"]
        source = Path(__file__).resolve().parents[1] / "tests/web_lifecycle.rs"
        assert hashlib.sha256(source.read_bytes()).hexdigest() == build["testSHA256"]
        self.freeze_source(source)
        self.freeze_source(Path(__file__).with_name("build_device_tests.py"))
        root = self.private / "sdk-lifecycle"
        root.mkdir(mode=0o700)
        environment = isolated_environment()
        environment.update(M119_WEB_PRIVATE=str(root), M119_WEB_LOCAL_RELAY=json.loads(self.configuration)["relays"][0])
        result = run_process([str(binary), "--exact", "owned_web_future_cancel_close_drop_and_reopen_are_bounded",
                              "--nocapture", "--test-threads=1"], env=environment, timeout=120, owners=self.nodes,
                             receipt=self.output / "sdk-consumer-owner.jsonl")
        (self.output / "sdk.stdout").write_text(result.stdout)
        (self.output / "sdk.stderr").write_text(result.stderr)
        self.record("controller", "public_web_sdk_consumer", {"exit": result.returncode, "pin_sha256": build["sha256"],
                    "source_receipt": str(self.args.consumer_receipt), "SDK_receipt": build["SDKReceipt"]})
        assert result.returncode == 0, "public Web SDK lifecycle consumer failed; raw primary/cleanup assertions retained"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--consumer-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--case", default="lifecycle")
    SDKRunner(parser.parse_args()).run()
