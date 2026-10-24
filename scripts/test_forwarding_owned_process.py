#!/usr/bin/env python3
"""Ordinary owned-child signal safety: durable intent and changed identity."""
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from forwarding_local_resources import OwnedProcess

CHILD = """
import json,os,pathlib,signal,sys
receipt,result=map(pathlib.Path,sys.argv[1:])
def end(number,frame):
    rows=[json.loads(row) for row in receipt.read_text().splitlines()]
    present=any(row['phase']=='signal_intent' and row['identity']['pid']==os.getpid() and row['signal']==number for row in rows)
    result.write_text(json.dumps({'durable_intent_before_handler':present}))
    sys.exit(0)
signal.signal(signal.SIGTERM,end)
print('READY',flush=True)
sys.stdin.read()
"""


class OwnedProcessTests(unittest.TestCase):
    def check_child(self, changed_identity):
        with tempfile.TemporaryDirectory(prefix="m118-owned-child-") as directory:
            root = Path(directory)
            process = subprocess.Popen([sys.executable, "-u", "-c", CHILD, str(root / "owner.jsonl"), str(root / "child-result.json")],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            owner = None
            try:
                self.assertEqual(process.stdout.readline().strip(), "READY")
                owner = OwnedProcess(process, root / "owner.jsonl")
                expected = owner.expected.copy()
                if changed_identity:
                    owner.expected = {**expected, "start_ticks": "deliberately-different-owned-fixture"}
                    with self.assertRaisesRegex(RuntimeError, "identity changed"):
                        owner.send(signal.SIGTERM)
                    self.assertIsNone(process.poll())
                    self.assertFalse(any(row["phase"] == "signal_sent" for row in map(json.loads, (root / "owner.jsonl").read_text().splitlines())))
                    owner.expected = expected
                owner.send(signal.SIGTERM)
                self.assertEqual(process.wait(timeout=5), 0)
                self.assertTrue(json.loads((root / "child-result.json").read_text())["durable_intent_before_handler"])
            finally:
                if process.poll() is None and owner is not None:
                    owner.send(signal.SIGTERM)
                    process.wait(timeout=5)
                if owner is not None:
                    owner.close()
                for stream in [process.stdin, process.stdout, process.stderr]:
                    stream.close()

    def test_durable_intent_precedes_actual_owned_signal(self):
        self.check_child(False)

    def test_identity_mismatch_refuses_effect_and_still_cleans_owned_child(self):
        self.check_child(True)


if __name__ == "__main__":
    unittest.main()
