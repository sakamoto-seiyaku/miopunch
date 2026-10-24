#!/usr/bin/env python3
"""Controller first-failure retention with independent real local failures."""
from concurrent.futures import Future
from pathlib import Path
import tempfile
import unittest

from check_web_service import WebRunner
from web_process_owner import wait_future


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.runner = object.__new__(WebRunner)
        self.events = []
        self.runner.record = lambda node, operation, value: self.events.append((operation, value))

    def test_primary_survives_release_disable_and_join_failures(self):
        primary = AssertionError("owned original close budget failure")
        attempted = []
        failed = Future()
        failed.set_exception(TimeoutError("owned failed join result"))
        with tempfile.TemporaryDirectory(prefix="m119-cleanup-test-") as private:
            release = Path(private) / "missing-parent" / "release"
            def action(label, operation):
                def run():
                    attempted.append(label)
                    return operation()
                return run
            def rejected_disable():
                raise RuntimeError("owned disable failure")
            actions = [
                ("release", action("release", release.touch)),
                ("disable", action("disable", rejected_disable)),
                ("pending-join", action("pending-join", lambda: wait_future(failed, .1))),
                ("close-join", action("close-join", lambda: "owned last join observed")),
            ]
            with self.assertRaises(AssertionError) as caught:
                self.runner.finish_scenario(primary, actions)
        self.assertIs(caught.exception, primary)
        self.assertEqual(attempted, ["release", "disable", "pending-join", "close-join"])
        result = self.events[-1][1]
        self.assertEqual([row["step"] for row in result["errors"]], ["release", "disable", "pending-join"])
        self.assertEqual(result["outcomes"][-1]["result"], "owned last join observed")
        self.assertIn("Independent cleanup failures", primary.__notes__[0])

    def test_cleanup_failure_without_primary_still_attempts_later_steps(self):
        attempted = []
        def failed():
            raise ValueError("owned cleanup failure")
        with self.assertRaisesRegex(RuntimeError, "Independent scenario cleanup failed"):
            self.runner.finish_scenario(None, [("first", failed), ("last", lambda: attempted.append("last"))])
        self.assertEqual(attempted, ["last"])
        self.assertEqual(self.events[-1][1]["errors"][0]["type"], "ValueError")

    def test_journal_failure_cannot_replace_the_primary(self):
        primary = RuntimeError("owned primary")
        def failed_record(*_arguments):
            raise OSError("owned journal failure")
        self.runner.record = failed_record
        with self.assertRaises(RuntimeError) as caught:
            self.runner.finish_scenario(primary, [("last", lambda: True)])
        self.assertIs(caught.exception, primary)
        self.assertIn("cleanup-journal", primary.__notes__[0])

    def test_success_records_each_independent_outcome(self):
        result = self.runner.finish_scenario(None, [("release", lambda: True), ("join", lambda: 0)])
        self.assertEqual(result, [{"step":"release", "result":True}, {"step":"join", "result":0}])
        self.assertEqual(self.events[-1][1]["errors"], [])


if __name__ == "__main__":
    unittest.main()
