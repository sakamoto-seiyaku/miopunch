#!/usr/bin/env python3
"""Controller failures preserve the original error and all independent cleanup.

These ordinary Python cases make no SDK/platform/terminal qualification claim.
"""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from check_terminal_service import TerminalRunner, WorkspaceObserver


class Owner:
    def __init__(self, events, name, close_failure=False, terminal_failure=False):
        self.events, self.name = events, name
        self.close_failure, self.terminal_failure = close_failure, terminal_failure

    def close(self):
        self.events.append("close:" + self.name)
        if self.close_failure:
            raise RuntimeError("owned cleanup failed")
        return {"owner":self.name,"exit":0}

    def terminal(self):
        self.events.append("terminal:" + self.name)
        if self.terminal_failure:
            raise RuntimeError("terminal observation failed")
        return True


class CleanupTests(unittest.TestCase):
    def run_failure(self, *, cleanup=False, observe=False, journal=False):
        with tempfile.TemporaryDirectory() as directory:
            runner = TerminalRunner.__new__(TerminalRunner)
            runner.output = Path(directory) / "receipt"
            runner.output.mkdir()
            runner.private = Path(directory) / "private"
            runner.private.mkdir()
            (runner.private / ".owner").write_text(str(runner.output))
            runner.controller_sources = {}
            events = []
            runner.nodes = [Owner(events, "first", close_failure=cleanup), Owner(events, "second", terminal_failure=observe)]
            runner.targets = []
            primary = ValueError("original terminal business failure")
            def scenario():
                raise primary
            runner.scenario = scenario
            def record(*args):
                if journal:
                    raise OSError("cleanup journal failed")
            runner.record = record
            with self.assertRaises(ValueError) as caught:
                runner.run()
            self.assertIs(caught.exception, primary)
            self.assertEqual(set(events), {"close:first", "close:second", "terminal:first", "terminal:second"})
            report = json.loads((runner.output / "result.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["failure"], str(primary))
            self.assertEqual(len(report["cleanup"]), 2)
            if cleanup or observe:
                self.assertTrue(runner.private.exists())
            if observe:
                self.assertFalse(report["all_owned_terminal"])

    def test_original_failure_survives_cleanup(self):
        self.run_failure(cleanup=True)

    def test_original_failure_survives_terminal_observation(self):
        self.run_failure(observe=True)

    def test_original_failure_survives_cleanup_journal(self):
        self.run_failure(journal=True)

    def test_all_independent_failures_preserve_original(self):
        self.run_failure(cleanup=True, observe=True, journal=True)

    def test_tmux_observation_failure_retains_unknown_cleanup_and_private(self):
        with tempfile.TemporaryDirectory() as directory:
            runner = TerminalRunner.__new__(TerminalRunner)
            runner.output = Path(directory) / "receipt"
            runner.output.mkdir()
            runner.private = Path(directory) / "private"
            runner.private.mkdir()
            (runner.private / ".owner").write_text(str(runner.output))
            runner.controller_sources, runner.nodes, runner.targets = {}, [], []
            observer = WorkspaceObserver(runner, runner.private)
            observer.socket.touch()
            primary = ValueError("original pressure premise failure")
            def scenario():
                raise primary
            runner.scenario, runner.record = scenario, lambda *args: None
            with patch("check_terminal_service.tmux_command", side_effect=RuntimeError("server exited unexpectedly")):
                with self.assertRaises(ValueError) as caught:
                    runner.run()
            self.assertIs(caught.exception, primary)
            report = json.loads((runner.output / "result.json").read_text())
            self.assertFalse(report["all_owned_terminal"])
            self.assertIn("WorkspaceObserver", report["unsettled"])
            self.assertFalse(report["cleanup"][0]["terminal"])
            self.assertTrue(report["cleanup"][0]["settlement_unknown"])
            self.assertTrue(report["cleanup"][0]["errors"])
            self.assertTrue(runner.private.exists())


if __name__ == "__main__":
    unittest.main()
