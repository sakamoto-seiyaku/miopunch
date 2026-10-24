#!/usr/bin/env python3
"""Real own-child launch, early-exit and guarded identity-transition tests."""
import json
from concurrent.futures import Future
import signal
import subprocess
import sys
import tempfile
import time
import threading
import unittest
from pathlib import Path

from check_forwarding import isolated_environment
from web_process_owner import OrdinaryChild, register_process, run_process, wait_future


class LaunchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="m119-owner-test-")
        self.receipt = Path(self.directory.name) / "owner.jsonl"
        self.owners = []

    def tearDown(self):
        for owner in self.owners:
            owner.close()
            self.assertTrue(owner.terminal())
        self.directory.cleanup()

    def test_ordinary_helper_is_observed_before_normal_completion(self):
        result = run_process([sys.executable, "-c", "import sys; print(sys.stdin.read())"],
            env=isolated_environment(), receipt=self.receipt, owners=self.owners, timeout=3, input="owned stdin")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "owned stdin\n")
        self.assertEqual(len(self.owners), 1)
        self.assertTrue(self.owners[0].terminal())

    def test_early_exit_still_has_terminal_registered_observation(self):
        try:
            run_process([sys.executable, "-c", "raise SystemExit(7)"], env=isolated_environment(),
                receipt=self.receipt, owners=self.owners, timeout=3)
        except RuntimeError:
            pass  # Early exit can occur before two launch observations.
        self.assertEqual(len(self.owners), 1)
        self.assertEqual(self.owners[0].process.returncode, 7)
        self.assertTrue(self.owners[0].terminal())

    def test_timeout_preserves_original_stdout_stderr_and_exception(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            run_process([sys.executable, "-c",
                "import sys,time;print('owned timeout stdout',flush=True);"
                "print('owned timeout stderr',file=sys.stderr,flush=True);time.sleep(.4)"],
                env=isolated_environment(), receipt=self.receipt, owners=self.owners, timeout=.1)
        self.assertTrue(self.owners[0].terminal())
        self.assertEqual(self.receipt.with_suffix(".stdout").read_text(), "owned timeout stdout\n")
        self.assertEqual(self.receipt.with_suffix(".stderr").read_text(), "owned timeout stderr\n")

    def test_wrong_expected_launch_remains_unqualified_and_own_eof_settles(self):
        process = subprocess.Popen([sys.executable, "-c", "import sys;print('owned unqualified stdout',flush=True);"
            "print('owned unqualified stderr',file=sys.stderr,flush=True);sys.stdin.read()"],
            env=isolated_environment(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        child = OrdinaryChild(process, self.receipt)
        self.owners.append(child)
        process.args = [sys.executable, "-c", "different expected launch"]
        with self.assertRaises(RuntimeError):
            register_process(process, self.receipt)
        self.assertIsNone(child.owner)
        child.close()
        self.assertEqual(process.returncode, 0)
        self.assertEqual(self.receipt.with_suffix(".stdout").read_text(), "owned unqualified stdout\n")
        self.assertEqual(self.receipt.with_suffix(".stderr").read_text(), "owned unqualified stderr\n")
        rows = [json.loads(line) for line in self.receipt.read_text().splitlines()]
        self.assertEqual(rows[0]['phase'], 'popen_created')
        self.assertEqual(rows[0]['pid'], process.pid)
        self.assertFalse(any(row['phase'] in {'registered', 'signal_sent'} for row in rows))

    def test_exec_after_registration_refuses_signal_and_own_stdin_closes(self):
        program = "import os,sys; sys.stdin.buffer.read(1); os.execv('/usr/bin/cat',['/usr/bin/cat'])"
        process = subprocess.Popen([sys.executable, "-c", program], env=isolated_environment(),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        owner = None
        try:
            owner = register_process(process, self.receipt)
            process.stdin.write(b"x")
            process.stdin.flush()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                if (Path('/proc') / str(process.pid) / 'cmdline').read_bytes() == b'/usr/bin/cat\0':
                    break
                time.sleep(.01)
            else:
                self.fail("owned exec did not reach actual new identity")
            with self.assertRaises(RuntimeError):
                owner.send(signal.SIGTERM)
            process.communicate(input=b"owned normal EOF", timeout=3)
            self.assertEqual(process.returncode, 0)
            rows = [json.loads(line) for line in self.receipt.read_text().splitlines()]
            self.assertTrue(any(row['phase'] == 'refused_identity_change' for row in rows))
            self.assertFalse(any(row['phase'] == 'signal_sent' for row in rows))
        finally:
            if process.poll() is None:
                process.communicate(input=b"", timeout=3)
            if owner is not None:
                owner.close()


class FutureWaitTests(unittest.TestCase):
    def test_owned_completion_is_observed_and_worker_joined(self):
        future = Future()
        def complete():
            time.sleep(.04)
            future.set_result("owned completion")
        worker = threading.Thread(target=complete)
        worker.start()
        try:
            self.assertEqual(wait_future(future, .5), "owned completion")
        finally:
            worker.join()
        self.assertFalse(worker.is_alive())

    def test_pending_future_keeps_original_monotonic_budget(self):
        future = Future()
        started = time.monotonic()
        with self.assertRaisesRegex(TimeoutError, "monotonic wait budget"):
            wait_future(future, .08)
        self.assertGreaterEqual(time.monotonic()-started, .08)
        self.assertFalse(future.done())
        future.cancel()

    def test_completed_future_preserves_original_task_exception(self):
        future = Future()
        original = TimeoutError("owned task failure, not a pending wait")
        future.set_exception(original)
        with self.assertRaises(TimeoutError) as caught:
            wait_future(future, .5)
        self.assertIs(caught.exception, original)


if __name__ == "__main__":
    unittest.main()
