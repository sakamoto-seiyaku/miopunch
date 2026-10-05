#!/usr/bin/env python3
"""Cache isolation and cleanup regressions; no Cargo compilation."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import cargo_cache as cache


class CargoCacheTests(unittest.TestCase):
    def test_per_case_target_requires_an_explicit_isolation_reason(self):
        root = Path('/tmp/owned-project')
        self.assertEqual(cache.target_directory(root), root / 'target')
        with self.assertRaises(cache.CacheError):
            cache.target_directory(root, '/tmp/new-target-for-each-case')
        self.assertEqual(cache.target_directory(root, '/tmp/isolated-target', 'different source projection'),
                         Path('/tmp/isolated-target'))

    def test_profile_and_incremental_overrides_cannot_silently_split_the_cache(self):
        for environment in ({'CARGO_PROFILE_DEV_DEBUG': '2'}, {'CARGO_PROFILE_TEST_DEBUG': '0'},
                            {'CARGO_INCREMENTAL': '1'}):
            with self.subTest(environment=environment), self.assertRaises(cache.CacheError):
                cache.build_environment(Path('/tmp/shared'), environment)
        self.assertEqual(cache.build_environment(Path('/tmp/shared'), {})['CARGO_INCREMENTAL'], '0')

    def test_low_disk_or_oversized_cache_refuses_before_compilation(self):
        with tempfile.TemporaryDirectory() as directory:
            for state in ({'freeBytes': 1, 'cacheBytes': 0},
                          {'freeBytes': cache.MIN_FREE, 'cacheBytes': cache.MAX_CACHE + 1}):
                entered = False
                with patch.object(cache, 'disk_state', return_value=state), self.assertRaises(cache.CacheError):
                    with cache.cache_lease(Path(directory)):
                        entered = True
                self.assertFalse(entered)

    def test_incremental_cleanup_preserves_compiled_artifacts_and_external_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'target'
            incremental = target / 'debug/incremental'
            incremental.mkdir(parents=True)
            (incremental / 'scratch').write_text('regenerable IR')
            artifact = target / 'debug/owned-sdk'
            artifact.write_text('retained compiled SDK')
            external = root / 'retained'
            external.mkdir()
            (external / 'proof').write_text('retained evidence')
            cross = target / 'wasm32-unknown-unknown/debug'
            cross.mkdir(parents=True)
            (cross / 'incremental').symlink_to(external, target_is_directory=True)
            with patch.object(cache, 'active_compilers', return_value=[]):
                report = cache.cleanup_incremental(target, root)
            self.assertFalse(incremental.exists())
            self.assertEqual(artifact.read_text(), 'retained compiled SDK')
            self.assertEqual((external / 'proof').read_text(), 'retained evidence')
            self.assertEqual(len(report['removedIncrementalCaches']), 1)

    def test_test_filter_does_not_receive_cargo_json_flag(self):
        target = Path('/tmp/shared')
        command = cache.cargo_command(['test', '--lib', '--', '--exact', 'one_test'], target)
        self.assertLess(command.index('--message-format=json'), command.index('--'))
        with self.assertRaises(cache.CacheError):
            cache.cargo_command(['build', '--target-dir=/tmp/clone'], target)

    def test_incremental_cleanup_does_not_follow_symlinked_profile_ancestors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'target'
            target.mkdir()
            for profile in (target / 'debug', target / 'wasm32-unknown-unknown/debug'):
                with self.subTest(profile=profile):
                    external = root / ('retained-' + profile.parent.name)
                    incremental = external / 'incremental'
                    incremental.mkdir(parents=True)
                    proof = incremental / 'proof'
                    proof.write_text('retained external evidence')
                    profile.parent.mkdir(parents=True, exist_ok=True)
                    profile.symlink_to(external, target_is_directory=True)
                    with patch.object(cache, 'active_compilers', return_value=[]):
                        report = cache.cleanup_incremental(target, root)
                    self.assertEqual(report['removedIncrementalCaches'], [])
                    self.assertEqual(proof.read_text(), 'retained external evidence')
                    profile.unlink()


if __name__ == '__main__':
    unittest.main()
