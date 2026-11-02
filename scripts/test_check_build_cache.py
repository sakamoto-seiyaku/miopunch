"""Build/run boundary regressions using owned files; never invokes Cargo."""
import contextlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import build_native_client
import cargo_cache
import check_build_cache as checks
from pin_artifact import pin_artifact as real_pin


class CheckBatch(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="miopunch-check-batch-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in ["Cargo.lock", ".cargo/config.toml", "src/lib.rs", "scripts/build_native_client.py",
                     "scripts/cargo_cache.py", "scripts/pin_artifact.py", "scripts/check_build_cache.py"]:
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("frozen fixture\n")
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        for obj, field in [(cargo_cache, "ROOT"), (build_native_client, "ROOT")]:
            p = patch.object(obj, field, self.root)
            p.start(); self.addCleanup(p.stop)
        self.compiler_calls, self.runtime_calls = [], []
        self.compiler_exit = 0
        self.batch = checks.CheckBuilds(self.root / "receipts")
        p = patch.object(cargo_cache, "cache_lease", side_effect=lambda *a, **k: contextlib.nullcontext({"cacheBytes": 0, "freeBytes": 20 * 1024**3}))
        p.start(); self.addCleanup(p.stop)
        p = patch.object(cargo_cache, "disk_state", return_value={"cacheBytes": 0, "freeBytes": 20 * 1024**3})
        p.start(); self.addCleanup(p.stop)
        p = patch.object(checks, "pin_artifact", side_effect=lambda source, destination: real_pin(
            source, destination, store=self.root / ".artifacts", min_free=0))
        p.start(); self.addCleanup(p.stop)
        p = patch.object(checks.subprocess, "Popen", side_effect=self.compiler)
        p.start(); self.addCleanup(p.stop)
        p = patch.object(checks.subprocess, "run", side_effect=self.runtime)
        p.start(); self.addCleanup(p.stop)

    def compiler(self, command, **kwargs):
        self.compiler_calls.append((command, kwargs))
        names = [command[i + 1] for i, arg in enumerate(command[:-1]) if arg == "--test"]
        entries = []
        for name, is_test in [(name, True) for name in names] + [("miopunch-native-probe", False)]:
            program = self.root / "target/debug" / ("deps/" + name if is_test else name)
            program.parent.mkdir(parents=True, exist_ok=True)
            program.write_bytes((name + repr(command)).encode())
            program.chmod(0o755)
            entries.append(dict(reason="compiler-artifact", package_id="fixture", target={"name": name, "kind": ["test" if is_test else "bin"]},
                                executable=str(program), filenames=[str(program)], profile={"test": is_test}, fresh=False))
        stdout = "\n".join(map(json.dumps, entries)) + "\n"
        code = self.compiler_exit

        class Process:
            returncode = code

            def communicate(self, timeout=None):
                return stdout, "original compiler failure\n" if code else ""

        return Process()

    def runtime(self, command, **kwargs):
        self.runtime_calls.append((command, kwargs))
        failed = "fails" in command
        return subprocess.CompletedProcess(command, 101 if failed else 0,
                                           "test result: FAILED. 0 passed; 1 failed\n" if failed else "test result: ok. 1 passed; 0 failed\n",
                                           "original assertion failure\n" if failed else "")

    def run_test(self, target, case="one"):
        return self.batch.run(["cargo", "test", "--test", target, case, "--", "--exact", "--nocapture"],
                              capture_output=True, text=True)

    def test_batch_unions_targets_and_runs_filters_without_more_cargo(self):
        self.batch.plan([("first", ["cargo", "test", "--test", "admission_gate", "one"]),
                         ("second", ["cargo", "test", "--test", "native_network", "two"])])
        self.assertEqual(self.run_test("admission_gate").returncode, 0)
        self.assertEqual(self.run_test("native_network", "two").returncode, 0)
        self.assertEqual(len(self.compiler_calls), 1)
        self.assertEqual(len(self.runtime_calls), 2)
        command = self.compiler_calls[0][0]
        self.assertIn("--no-run", command)
        self.assertNotIn("one", command)
        self.assertNotIn("two", command)
        self.assertTrue(self.runtime_calls[1][1]["env"]["MIOPUNCH_TEST_CLI"].startswith(str(self.root / ".artifacts")))
        self.assertTrue(self.runtime_calls[1][0][2].startswith(str(self.root / ".artifacts")))

    def test_reusing_a_different_feature_graph_restores_its_cli_pin(self):
        command = ["cargo", "test", "--test", "native_network", "one", "--features", "replica-test-hooks"]
        self.run_test("native_network")
        first = self.runtime_calls[-1][1]["env"]["MIOPUNCH_TEST_CLI"]
        self.batch.run(command, capture_output=True, text=True)
        second = self.runtime_calls[-1][1]["env"]["MIOPUNCH_TEST_CLI"]
        self.assertNotEqual(first, second)
        self.run_test("native_network")
        self.assertEqual(first, self.runtime_calls[-1][1]["env"]["MIOPUNCH_TEST_CLI"])
        self.assertEqual(len(self.compiler_calls), 2)

    def test_source_change_rejects_before_starting_cargo(self):
        self.batch._source_guard()
        (self.root / "src/new.rs").write_text("// newly added input\n")
        with self.assertRaisesRegex(cargo_cache.CacheError, "inputs changed"):
            self.run_test("admission_gate")
        self.assertEqual(self.compiler_calls, [])

    def test_changed_pin_rejects_before_another_runtime(self):
        self.run_test("admission_gate")
        pin = Path(self.runtime_calls[0][0][2])
        pin.chmod(0o755)  # Only the test's owned temporary store is mutated.
        pin.write_bytes(b"changed fixture")
        with self.assertRaisesRegex(cargo_cache.CacheError, "pin changed"):
            self.run_test("admission_gate", "two")
        self.assertEqual(len(self.compiler_calls), 1)
        self.assertEqual(len(self.runtime_calls), 1)

    def test_compile_failure_keeps_raw_exit_and_does_not_run(self):
        self.compiler_exit = 101
        result = self.run_test("admission_gate")
        self.assertEqual(result.returncode, 101)
        self.assertIn("original compiler failure", result.stderr)
        self.assertEqual(self.runtime_calls, [])
        receipt = json.loads((self.root / "receipts/builds/001/receipt.json").read_text())
        self.assertEqual(receipt["exit"], 101)
        self.assertTrue((self.root / "receipts/builds/001/cargo.jsonl").is_file())

    def test_runtime_failure_is_not_relabelled_as_build_success(self):
        result = self.run_test("admission_gate", "fails")
        self.assertEqual(result.returncode, 101)
        self.assertIn("original assertion failure", result.stderr)

    def test_stripping_uses_and_removes_only_a_deployment_copy(self):
        self.run_test("admission_gate")
        pin = Path(self.runtime_calls[0][0][2])
        before = pin.read_bytes(), pin.stat().st_mode
        with patch.object(checks.shutil, "which", return_value="/usr/bin/strip"):
            with self.assertRaisesRegex(RuntimeError, "runtime owner failed"):
                with self.batch.deployment(pin, strip_debug=True) as deployed:
                    self.assertNotEqual(pin.stat().st_ino, deployed.stat().st_ino)
                    self.assertEqual(self.runtime_calls[-1][0][-1], str(deployed))
                    raise RuntimeError("runtime owner failed")
        self.assertFalse(deployed.exists())
        self.assertEqual((pin.read_bytes(), pin.stat().st_mode), before)

    def test_rustc_overrides_are_not_silently_treated_as_test_filters(self):
        with self.assertRaisesRegex(cargo_cache.CacheError, "committed compiler settings"):
            self.batch.run(["cargo", "rustc", "--test", "tcp_direct", "--", "-C", "debuginfo=0"])
        self.assertEqual(self.compiler_calls, [])

    def test_package_selection_separates_tools_and_forwards_workspace_features(self):
        p = self.root / "tools/native/Cargo.toml"
        p.parent.mkdir(parents=True)
        p.write_text('[features]\nrtc-probe = []\n[[bin]]\nname = "browser_wss_member_native"\n')
        command = cargo_cache.cargo_command(["build", "--bin", "browser_wss_member_native"], self.root / "target")
        self.assertEqual(command[command.index("--package") + 1], "miopunch-native-tools")
        command = cargo_cache.cargo_command(["test", "--workspace", "--features", "rtc-probe"], self.root / "target")
        features = command[command.index("--features") + 1].split(",")
        self.assertEqual(set(features), {"miopunch-native-probe/rtc-probe", "miopunch-native-tools/rtc-probe"})


if __name__ == "__main__":
    unittest.main()
