#!/usr/bin/env python3
"""Execute current public host/device effects in an owned local namespace."""
import argparse
import hashlib
import json
from pathlib import Path

from check_forwarding import isolated_environment
from check_web_service import WebRunner
from forwarding_local_resources import configure_local_resources
from web_process_owner import register_process, run_process

CASES = {
    "custody": "stopped_custody_wire_delivery_never_restores_the_saved_root",
    "backup": "composed_backup_cannot_apply_policy_before_or_after_current_demotion",
    "host": "native_host_retains_identity_excludes_duplicate_and_closes_pending_work",
}


class EffectRunner(WebRunner):
    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        build = json.loads(self.args.consumer_receipt.read_text())
        binary = Path(build["pin"])
        assert build["exit"] == 0 and not build["CargoInvoked"] and build["dependencyRebuilds"] == 0
        assert Path(build["SDKReceipt"]) == self.args.source_receipt.resolve()
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["sha256"]
        source = Path(__file__).resolve().parents[1] / ("tests/client_host.rs" if self.case == "host" else "tests/device_effects.rs")
        assert hashlib.sha256(source.read_bytes()).hexdigest() == build["testSHA256"]
        self.freeze_source(source)
        self.freeze_source(Path(__file__).with_name("build_device_tests.py"))
        private = self.private / "device-effects"
        private.mkdir(mode=0o700)
        environment = isolated_environment()
        prefix = "M121_HOST" if self.case == "host" else "M116_EFFECT"
        environment.update({prefix + "_PRIVATE": str(private), prefix + "_LOCAL_RELAY": json.loads(self.configuration)["relays"][0]})
        result = run_process([str(binary), "--exact", CASES[self.case], "--nocapture", "--test-threads=1"],
                             env=environment, timeout=120, owners=self.nodes, receipt=self.output / "consumer-owner.jsonl")
        (self.output / "consumer.stdout").write_text(result.stdout)
        (self.output / "consumer.stderr").write_text(result.stderr)
        self.record("controller", "device_effect_consumer", {"case": self.case, "exit": result.returncode,
                    "consumer_pin_sha256": build["sha256"], "source_receipt": build["SDKReceipt"]})
        assert result.returncode == 0, "actual device effect failed; original evidence retained"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for argument in ["binary", "source-receipt", "consumer-receipt", "output", "relay-binary", "relay-receipt"]:
        parser.add_argument("--" + argument, type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--case", choices=list(CASES), required=True)
    EffectRunner(parser.parse_args()).run()
