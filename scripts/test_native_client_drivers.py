#!/usr/bin/env python3
"""Regression checks for the build preflight and independent owned cleanup."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build_native_client as build
from check_native_client import Runner


class BuildPreflight(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="m112-preflight-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        names = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "src/lib.rs",
                 "src/native_client.rs", "src/bin/miopunch-client.rs",
                 "scripts/build_native_client.py", "scripts/cargo_cache.py", "scripts/pin_artifact.py",
                 "browser-probe/Cargo.toml",
                 "vendor/sdk/Cargo.toml", "vendor/sdk/build.rs", "vendor/sdk/src/lib.rs",
                 "vendor/sdk/data/protocol.bin"]
        self.expected = {}
        for name in names:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"frozen fixture\n")
            self.expected[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest = self.root / "Cargo.toml"
        manifest.write_text('[workspace]\nmembers = ["browser-probe"]\n')
        self.expected["Cargo.toml"] = hashlib.sha256(manifest.read_bytes()).hexdigest()
        self.inputs = Path(self.temporary.name) / "inputs.json"
        self.output = Path(self.temporary.name) / "receipt"
        self.root_patch = patch.object(build, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def run_preflight(self):
        self.inputs.write_text(json.dumps(self.expected))
        args = ["build_native_client.py", "--platform", "linux", "--inputs", str(self.inputs),
                "--output", str(self.output), "--validate-only"]
        with patch.object(sys, "argv", args), patch.object(build, "cache_lease", side_effect=AssertionError("Cargo lease invoked")), \
                patch.object(build.subprocess, "check_output", return_value="rustc 1.98.1 test\n"), \
                contextlib.redirect_stdout(io.StringIO()) as stdout:
            build.main()
        return json.loads(stdout.getvalue())

    def test_complete_graph_validates_without_compiling_or_creating_receipt(self):
        self.assertFalse(self.run_preflight()["cargoInvoked"])
        self.assertFalse(self.output.exists())

    def test_omitted_sdk_source_or_asset_rejected_before_cargo(self):
        for name in ["src/lib.rs", "vendor/sdk/src/lib.rs", "vendor/sdk/data/protocol.bin", ".cargo/config.toml", "browser-probe/Cargo.toml"]:
            digest = self.expected.pop(name)
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, "omits"):
                self.run_preflight()
            self.assertFalse(self.output.exists())
            self.expected[name] = digest

    def test_new_module_after_freeze_rejected_before_cargo(self):
        (self.root / "src/new_module.rs").write_text("// added after graph capture\n")
        with self.assertRaisesRegex(RuntimeError, "src/new_module.rs"):
            self.run_preflight()
        self.assertFalse(self.output.exists())

    def test_lock_drift_rejected_before_cargo(self):
        (self.root / "Cargo.lock").write_text("changed graph\n")
        with self.assertRaisesRegex(RuntimeError, "Cargo.lock"):
            self.run_preflight()
        self.assertFalse(self.output.exists())


class CleanupIsolation(unittest.TestCase):
    def test_failed_scenario_and_local_stop_still_check_remote_and_publish_result(self):
        with tempfile.TemporaryDirectory(prefix="m112-cleanup-") as temporary:
            output = Path(temporary)
            state = output / "private-state"
            state.mkdir()
            invitation = output / "test-invite.json"
            invitation.write_text("private fixture")
            process = subprocess.Popen([sys.executable, "-c", "import sys; sys.stdin.read()"], stdin=subprocess.PIPE)
            self.addCleanup(lambda: process.kill() if process.poll() is None else None)
            stderr = (output / "stderr").open("w")

            class FailedHost:
                label = "owned-test-child"

                def stop(self):
                    raise RuntimeError("injected stop failure")

            host = FailedHost()
            host.process, host.stderr = process, stderr
            runner = Runner.__new__(Runner)
            runner.output, runner.hosts, runner.events = output, [host], []
            runner.remote_owned = True
            runner.remote_root = "C:/owned-fixture"
            runner.remote_binary = runner.remote_root + "/client.exe"
            calls = []

            def remote(script):
                calls.append(script)
                return '{"forced":0,"remaining":0}' if "ConvertTo-Json" in script else ""

            runner.remote = remote
            runner.local_only = True
            runner.deploy = lambda: None

            def fail_scenario(_remote):
                raise RuntimeError("injected scenario failure")

            runner.invalid_invitation = fail_scenario
            with self.assertRaisesRegex(RuntimeError, "scenario failure"):
                runner.run()
            report = json.loads((output / "result.json").read_text())
            self.assertFalse(report["passed"])
            results = report["cleanup"]
            self.assertIsNotNone(process.poll())
            process.stdin.close()
            self.assertEqual(len(calls), 2, "remote check and deployment removal must both run")
            self.assertTrue(any(entry.get("action") == "owned_process_check" for entry in results))
            self.assertTrue(any(entry.get("error") == "RuntimeError" for entry in results))
            self.assertFalse(invitation.exists())
            self.assertTrue(state.exists(), "private diagnosis state must be retained")

    def test_cleanup_never_touches_remote_directory_without_ownership(self):
        with tempfile.TemporaryDirectory(prefix="m112-cleanup-") as temporary:
            runner = Runner.__new__(Runner)
            runner.output, runner.hosts, runner.events = Path(temporary), [], []
            runner.remote_owned = False

            def remote(_script):
                raise AssertionError("unowned remote directory touched")

            runner.remote = remote
            self.assertEqual(runner.cleanup(), [])

    def test_remote_check_failure_does_not_skip_deployment_removal(self):
        with tempfile.TemporaryDirectory(prefix="m112-cleanup-") as temporary:
            runner = Runner.__new__(Runner)
            runner.output, runner.hosts, runner.events = Path(temporary), [], []
            runner.remote_owned, runner.remote_root, runner.remote_binary = True, "C:/owned", "C:/owned/client.exe"
            calls = []

            def remote(script):
                calls.append(script)
                if len(calls) == 1:
                    raise subprocess.CalledProcessError(1, "injected remote check")
                return ""

            runner.remote = remote
            results = runner.cleanup()
            self.assertEqual(len(calls), 2)
            self.assertEqual(results[0]["action"], "owned_process_check")
            self.assertEqual(results[0]["error"], "CalledProcessError")


if __name__ == "__main__":
    unittest.main()
