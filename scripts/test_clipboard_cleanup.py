#!/usr/bin/env python3
"""Failure-oracle checks using ordinary owned children; no clipboard or network."""
import json
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

from check_clipboard import Host, Runner


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="miopunch-clipboard-cleanup-oracle-"))
        self.addCleanup(shutil.rmtree, self.root)
        self.runner = Runner.__new__(Runner)
        self.runner.output = self.root / "receipt"
        self.runner.output.mkdir()
        self.runner.private = self.root / "private"
        self.runner.private.mkdir()
        (self.runner.private / ".owner").write_text(str(self.runner.output))
        self.runner.hosts, self.runner.desktops = [], []
        self.runner.controller_sources = {}
        self.runner.scenario = lambda: None

    def child(self, name, code):
        host = Host.__new__(Host)
        host.label, host.runner = name, self.runner
        host.log = (self.root / (name + ".log")).open("w")
        host.process = subprocess.Popen([sys.executable, "-c", "raise SystemExit(" + str(code) + ")"],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=host.log, text=True)
        host.lines = queue.Queue()
        host.reader = threading.Thread(target=host._read)
        host.reader.start()
        host.process.wait(timeout=5)
        return host

    def result(self):
        return json.loads((self.runner.output / "result.json").read_text())

    def test_nonzero_owned_host_fails_receipt_and_still_joins_other_host(self):
        good, bad = self.child("good", 0), self.child("bad", 7)
        self.runner.hosts = [good, bad]
        with self.assertRaisesRegex(AssertionError, "cleanup"):
            self.runner.run()
        result = self.result()
        self.assertEqual(result["status"], "failed")
        self.assertTrue(result["all_owned_processes_terminal"])
        self.assertTrue(result["private_state_removed"])
        self.assertEqual([entry["exit"] for entry in result["cleanup"]], [7, 0])
        self.assertTrue(all(entry["reader_joined"] for entry in result["cleanup"]))

    def test_desktop_exception_continues_other_cleanup_and_retains_primary_failure(self):
        observed = []

        class OwnedDesktop:
            process = None

            def __init__(self, name, fail=False):
                self.name, self.fail = name, fail

            def close(self):
                observed.append(self.name)
                if self.fail:
                    raise RuntimeError("controlled cleanup failure")

            def terminal(self):
                return True

            def release_lease(self):
                observed.append(self.name + "-lease")

        self.runner.desktops = [OwnedDesktop("remaining"), OwnedDesktop("failing", True)]

        def primary():
            raise ValueError("original operation failed")

        self.runner.scenario = primary
        with self.assertRaisesRegex(ValueError, "original operation"):
            self.runner.run()
        self.assertEqual(observed, ["failing", "failing-lease", "remaining", "remaining-lease"])
        self.assertEqual(self.result()["failure_type"], "ValueError")
        self.assertEqual(self.result()["status"], "failed")
        self.assertTrue(self.result()["cleanup_failures"])

    def test_unaccounted_browser_actor_prevents_private_state_removal(self):
        class UnaccountedBrowser:
            cleanup_result = {"client": "browser", "all_actor_processes_terminal": False}

            def close(self):
                raise RuntimeError("actor terminal proof unavailable")

            def terminal(self):
                return False

        self.runner.browser = UnaccountedBrowser()
        with self.assertRaises(AssertionError):
            self.runner.run()
        self.assertEqual(self.result()["status"], "failed")
        self.assertFalse(self.result()["all_owned_processes_terminal"])
        self.assertTrue(self.runner.private.exists())

    def test_missing_private_owner_keeps_state_and_writes_failed_receipt(self):
        (self.runner.private / ".owner").unlink()
        with self.assertRaises(AssertionError):
            self.runner.run()
        result = self.result()
        self.assertEqual(result["status"], "failed")
        self.assertTrue(result["all_owned_processes_terminal"])
        self.assertFalse(result["private_state_removed"])
        self.assertIn("FileNotFoundError", result["cleanup_failures"])


if __name__ == "__main__":
    unittest.main()
