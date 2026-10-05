#!/usr/bin/env python3
"""Regression for per-receipt linker paths invalidating Android Cargo caches."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_android_pair as builder


class AndroidLinkerCacheTests(unittest.TestCase):
    def test_new_receipt_directory_reuses_same_linker_without_rewriting_it(self):
        with tempfile.TemporaryDirectory(prefix="miopunch-linker-cache-test-") as temporary:
            root = Path(temporary)
            ndk = root / "ndk"
            windows = ndk / "toolchains/llvm/prebuilt/windows-x86_64"
            (windows / "sysroot").mkdir(parents=True)
            (windows / "lib/clang/21/lib/linux/aarch64").mkdir(parents=True)
            (ndk / "source.properties").write_text("Pkg.Revision = 29.0.14206865\n")
            sysroot = root / "rust"
            lld = sysroot / "lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld"
            lld.parent.mkdir(parents=True)
            lld.write_bytes(b"owned fake linker")
            clang, archiver = root / "clang", root / "llvm-ar"
            clang.write_bytes(b"owned fake clang")
            archiver.write_bytes(b"owned fake archiver")
            first, second = root / "receipt-one", root / "receipt-two"
            first.mkdir()
            second.mkdir()

            def capture(arguments):
                return str(sysroot) if "--print" in arguments else "host: x86_64-unknown-linux-gnu"

            with patch.object(builder, "ROOT", root), \
                 patch.object(builder, "capture", side_effect=capture), \
                 patch.object(builder.shutil, "which", side_effect=lambda name: str(clang if name == "clang" else archiver)):
                initial, initial_ar = builder.compiler(ndk, first)
                contents, mtime = initial.read_bytes(), initial.stat().st_mtime_ns
                reused, reused_ar = builder.compiler(ndk, second)
            self.assertEqual(initial, reused)
            self.assertEqual(initial_ar, reused_ar)
            self.assertEqual(reused.read_bytes(), contents)
            self.assertEqual(reused.stat().st_mtime_ns, mtime)
            self.assertFalse(initial.is_relative_to(first))
            self.assertFalse(initial.is_relative_to(second))
            self.assertNotIn(str(first), contents.decode())
            self.assertNotIn(str(second), contents.decode())


if __name__ == "__main__":
    unittest.main()
