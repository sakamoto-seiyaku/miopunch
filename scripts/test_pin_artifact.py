"""Immutable pins must reuse bytes without sharing Cargo's mutable inode."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import pin_artifact as pins

from pin_artifact import pin_artifact


class ArtifactPinTests(unittest.TestCase):
    def test_repeated_pins_share_storage_but_mutable_source_does_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "cargo-output"
            source.write_bytes(b"first executable")
            source.chmod(0o755)
            first, second = root / "run-one/sdk", root / "run-two/sdk"
            for path in (first, second):
                pin_artifact(source, path, store=root / "store", min_free=0)
            self.assertTrue(first.samefile(second))
            self.assertFalse(first.samefile(source))
            source.write_bytes(b"new build output")
            self.assertEqual(first.read_bytes(), b"first executable")
            self.assertEqual(first.stat().st_mode & 0o777, 0o555)

    def test_existing_destination_and_corrupt_object_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, pin = root / "source", root / "pin"
            source.write_bytes(b"original")
            pin_artifact(source, pin, store=root / "store", min_free=0)
            other = root / "other"
            other.write_bytes(b"keep this")
            with self.assertRaises(FileExistsError):
                pin_artifact(source, other, store=root / "store", min_free=0)
            self.assertEqual(other.read_bytes(), b"keep this")
            pin.chmod(0o644)
            pin.write_bytes(b"corrupt")
            with self.assertRaises(RuntimeError):
                pin_artifact(source, root / "next", store=root / "store", min_free=0)
            self.assertFalse((root / "next").exists())

    def test_changing_source_is_not_installed_as_an_immutable_object(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.write_bytes(b"before")
            copy = pins.shutil.copyfileobj

            def changed(incoming, output):
                copy(incoming, output)
                source.write_bytes(b"after")

            with patch.object(pins.shutil, 'copyfileobj', side_effect=changed), self.assertRaises(RuntimeError):
                pin_artifact(source, root / "pin", store=root / "store", min_free=0)
            self.assertFalse((root / "pin").exists())
            self.assertEqual(list((root / "store/sha256").rglob('.pin-*')), [])

    def test_mutable_target_cannot_be_used_as_a_pin_destination_or_store(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.write_bytes(b"compiled")
            with patch.object(pins, 'ROOT', root):
                with self.assertRaises(RuntimeError):
                    pin_artifact(source, root / 'target/sdk', store=root / 'store', min_free=0)
                with self.assertRaises(RuntimeError):
                    pin_artifact(source, root / 'pin', store=root / 'target/store', min_free=0)

    def test_budget_failure_leaves_no_pin_and_symlink_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.write_bytes(b"larger than budget")
            with self.assertRaises(RuntimeError):
                pin_artifact(source, root / "pin", store=root / "store", min_free=0, max_bytes=1)
            self.assertFalse((root / "pin").exists())
            link = root / "symlink"
            link.symlink_to(source)
            with self.assertRaises(RuntimeError):
                pin_artifact(link, root / "pin", store=root / "store", min_free=0)


if __name__ == "__main__":
    unittest.main()
