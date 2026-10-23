#!/usr/bin/env python3
"""Real Browser policy receiver applies a signed StrongRemove during file I/O.

Native carriers use the public SDK/runtime-test pin. Root material belongs only
to this owned fixture and stays outside receipts. No prototype or transport hook.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from check_browser_services import BrowserServiceRunner
from check_browser_onboarding import BrowserControl
from check_native_client import Host


class CorrectionRunner(BrowserServiceRunner):
    def __init__(self, args):
        super().__init__(args)
        build = json.loads(args.runtime_receipt.read_text())
        assert build["exit"] == 0
        assert hashlib.sha256(args.runtime.read_bytes()).hexdigest() == build["sha256"]
        root = Path(__file__).resolve().parents[1]
        source = root / "tests/service_policy_runtime.rs"
        assert hashlib.sha256(source.read_bytes()).hexdigest() == build["testSHA256"]
        for source in [source, root / "scripts/build_service_runtime_tests.py"]:
            self.controller_sources[str(source)] = hashlib.sha256(source.read_bytes()).hexdigest()
            target = self.output / "controller-source" / source.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
        (self.output / "controller-sources.json").write_text(json.dumps(self.controller_sources, indent=2))

    def correction(self, label, carrier, provider, obsolete):
        config = self.private_root / (label + "-carrier.json")
        report = self.private_root / (label + "-carrier-result.json")
        config.write_text(json.dumps(dict(carrierNode=carrier.directory,
            recoveryCard=str(self.private_root / (label + "-root-card")), administrator=self.identity(label),
            receiver=self.address_of(label), incomingServices=provider.directory + "/services.records" if obsolete else None,
            report=str(report))))
        config.chmod(0o600)
        environment = dict(os.environ, MIOPUNCH_CORRECTION_CONFIG=str(config))
        with (self.output / (label + "-carrier.stdout")).open("w") as stdout, (self.output / (label + "-carrier.stderr")).open("w") as stderr:
            result = subprocess.run(["rtk", "proxy", str(self.args.runtime.resolve()), "--ignored", "--exact",
                "external_correction_carrier", "--nocapture"], env=environment, stdout=stdout, stderr=stderr, timeout=120)
        assert result.returncode == 0, "owned public SDK carrier failed; inspect retained runtime stderr"
        result = json.loads(report.read_text())
        self.record(label, "original_signed_correction_delivered", result)
        assert result["sent"]
        return result

    def scenario(self, obsolete):
        name = "wasm-corrected-" + ("obsolete" if obsolete else "retained")
        receiver = self.browser_creator(name)
        self.browser.call(receiver, "enable_files")
        card = self.private_root / (name + "-root-card")
        card.write_text(self.browser.call(receiver, "export_recovery_card"))
        card.chmod(0o600)
        carrier = self.native_member(receiver, name + "-common-carrier")
        carrier.stop()  # Its graph is the actual common predecessor of Add(P).
        provider = self.native_member(receiver, name + "-later-provider")
        self.enable(provider, name + "-provider")
        provider.call("service_sync", address=self.address_of(receiver))
        self.browser.call(receiver, "refresh_service_state")
        assert any(item["provider"] == provider.node and item["enabled"]
            for item in self.browser.call(receiver, "service_status")["declarations"])
        source, expected = self.generate(provider, name + "-baseline.bin", 11)
        self.saved_mixed(provider, receiver, self.send_mixed(provider, receiver, source), expected)
        if obsolete:
            provider.call("files_disable")  # New genuine signed head, not sent yet.
        before = set(json.loads(self.browser.call(receiver, "admission_material_receipt"))["records"])
        source, _ = self.generate(provider, name + "-live.bin", 512 * 1024**2)
        task = self.send_mixed(provider, receiver, source)
        self.wait_progress(provider, receiver, task)
        delivery = self.correction(receiver, carrier, provider, obsolete)
        self.browser.call(receiver, "refresh_service_state")
        after = set(json.loads(self.browser.call(receiver, "admission_material_receipt"))["records"])
        self.record(receiver, "post_correction_independent_observations", dict(newStoredRecords=sorted(after - before),
            receivingTask=self.task(receiver, task), sourceTask=self.task(provider, task), delivery=delivery))
        assert len(after - before) == 1, "valid correction did not reach the actual durable Browser snapshot"
        assert delivery["replyAccepted"] == (not obsolete)
        self.stopped(provider, receiver, task, "strong_remove_stops_actual_OPFS_file")
        assert self.browser.call(receiver, "file_status")["enabled"]
        self.browser.call(receiver, "set_service_rule", carrier.node, "files", None,
            json.dumps(dict(members=True, guests=True, allowed=[], denied=[])), ok=False)
        restored = Host(self, carrier.label + "-reopen", False, "open", carrier.directory)
        assert restored.node == carrier.node
        source, expected = self.generate(restored, name + "-post-correction.bin", 11)
        self.saved_mixed(restored, receiver, self.send_mixed(restored, receiver, source), expected)
        self.record(receiver, "actual_browser_management_correction", dict(retainedHistory=not obsolete,
            policyRejectedButManagementSaved=obsolete, publicStoredRecordAdded=True, administratorRoleLost=True,
            actualFileStoppedAndCleaned=True, ordinaryPostCorrectionSaveHash=True, noPrototypeHooks=True))
        provider.stop()
        restored.stop()

    def run(self):
        passed, browser_cleanup = False, None
        try:
            self.browser = BrowserControl(self, self.args)
            self.scenario(False)
            self.scenario(True)
            passed = True
        finally:
            if hasattr(self, "browser"):
                try:
                    browser_cleanup = self.browser.close()
                except Exception as error:
                    browser_cleanup = dict(error=type(error).__name__)
            cleanup = self.cleanup()
            drift = self.controller_drift()
            passed = passed and not drift and browser_cleanup is not None and "error" not in browser_cleanup and all("error" not in item for item in cleanup)
            (self.output / "result.json").write_text(json.dumps(dict(passed=passed, qualifications=self.qualifications,
                browserCleanup=browser_cleanup, cleanup=cleanup, controllerSourceDrift=drift,
                runtimePinSHA256=hashlib.sha256(self.args.runtime.read_bytes()).hexdigest(), wasmPackage=self.args.package_sha,
                noPrototypeHooks=True, rootFixtureMaterialRemoved=not self.private_root.exists()), indent=2))
        assert passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--runtime-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    args.harness = root / "browser-probe/harness/service-policy.mjs"
    build = json.loads(args.build_receipt.read_text())
    actual = {path.relative_to(args.package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in args.package.rglob("*") if path.is_file()}
    assert build["exit"] == build["bindgenExit"] == 0 and build["sourceFrozenDuringBuild"] and actual == build["artifacts"]
    args.package_sha = build["packageSHA256"]
    args.windows = args.windows_sha = None
    args.local_only, args.failure_only, args.focus = True, False, "full"
    CorrectionRunner(args).run()


if __name__ == "__main__":
    main()
