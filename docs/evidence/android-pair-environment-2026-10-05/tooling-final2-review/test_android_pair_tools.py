#!/usr/bin/env python3
"""Local ownership/failure receipts; no phone, public service or STUN operation."""
import copy
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

import android_pair_runtime as runtime
import build_android_pair as builder


class FakeDevice:
    def __init__(self, xmls=(), fail_remove=False):
        self.xmls = iter(xmls)
        self.calls = []
        self.fail_remove = fail_remove

    def shell(self, *args, **_):
        self.calls.append(args)
        return (next(self.xmls), 0) if args[0] == 'cat' else ('', 0)

    def run(self, *args, **_):
        self.calls.append(args)
        if self.fail_remove and args[:2] == ('forward', '--remove'):
            raise OSError('controlled removal failure')
        return '', 0


def ui(text, resource='com.android.chrome:id/url_bar', package='com.android.chrome'):
    root = ET.Element('hierarchy')
    ET.SubElement(root, 'node', text=text, **{'resource-id': resource, 'package': package})
    return ET.tostring(root, encoding='unicode')


def page_for_ui(xmls):
    page = runtime.BrowserPage.__new__(runtime.BrowserPage)
    page.device = FakeDevice(xmls)
    page.browser_version = None
    page.actor = 'owned-actor'
    page.port = 32123
    page.url = 'http://127.0.0.1:32123/android-pair?actor=owned-actor&environmentRtcProbe=true'
    page.cleanup = {}
    return page


class OwnershipTests(unittest.TestCase):
    def test_body_port_other_actor_and_shortened_bar_cannot_close(self):
        exact = page_for_ui(()).url
        for text, resource, package in (
                (exact, 'body_text', 'com.android.chrome'),
                (exact.replace('owned-actor', 'another-actor'), 'com.android.chrome:id/url_bar', 'com.android.chrome'),
                ('127.0.0.1:32123', 'com.android.chrome:id/url_bar', 'com.android.chrome'),
                (exact, 'com.android.chrome:id/url_bar', 'another.package')):
            with self.subTest(text=text, resource=resource, package=package):
                page = page_for_ui([ui(text, resource, package)])
                result = page._close_owned_tab()
                self.assertFalse(result['ownedTargetAbsent'])
                self.assertFalse(any(call[:2] == ('input', 'keycombination') for call in page.device.calls))

    def test_only_exact_owned_address_bar_can_send_close(self):
        page = page_for_ui(())
        page.device = FakeDevice([ui(page.url), ui('https://example.test/')])
        self.assertTrue(page._close_owned_tab()['ownedTargetAbsent'])
        self.assertEqual(sum(call[:2] == ('input', 'keycombination') for call in page.device.calls), 1)

    def test_missing_or_short_after_bar_is_not_absence_proof(self):
        for after in ('<hierarchy/>', ui('127.0.0.1:32123')):
            page = page_for_ui(())
            page.device = FakeDevice([ui(page.url), after])
            self.assertFalse(page._close_owned_tab()['ownedTargetAbsent'])

    def test_shutdown_timeout_still_closes_tab_rules_and_listener(self):
        page = runtime.BrowserPage.__new__(runtime.BrowserPage)
        page.ready, page.opened, page.closed = {'event': 'ready'}, True, False
        page.forward_attempted = True
        page.forward, page.reverse = 'tcp:32123', 'tcp:32124'
        page.cleanup = {}
        page.device = FakeDevice(fail_remove=True)
        calls = []
        class Thread:
            alive = True
            def join(self, **_): calls.append('join'); self.alive = False
            def is_alive(self): return self.alive
        class Socket:
            fd = 5
            def fileno(self): return self.fd
        class Server:
            socket = Socket()
            def shutdown(self): calls.append('shutdown')
            def server_close(self): calls.append('server_close'); self.socket.fd = -1
        page.thread, page.server = Thread(), Server()
        def timeout(*_, **__): raise TimeoutError('controlled SDK shutdown wait')
        page.call = timeout
        page._close_owned_tab = lambda: calls.append('tab') or {'ownedTargetAbsent': True}
        result = page.close()
        self.assertEqual(calls, ['tab', 'shutdown', 'server_close', 'join'])
        self.assertIn(('forward', '--list'), page.device.calls)
        self.assertIn(('reverse', '--remove', 'tcp:32124'), page.device.calls)
        self.assertIn(('reverse', '--list'), page.device.calls)
        self.assertFalse(runtime.browser_cleanup_succeeded(result))
        self.assertTrue(result['localListenerClosed'])

    def test_every_applicable_failure_or_missing_fact_fails(self):
        good = {'sdkRequired': True, 'tabRequired': True, 'forwardRequired': True, 'reverseRequired': True,
                'sdk': {'ok': True, 'result': {'cleanupSucceeded': True, 'networkClosed': True,
                    'networkFreed': True, 'ownedMemberHandles': 0, 'storage': {'required': False}}},
                'tab': {'ownedTargetAbsent': True},
                'forwardRuleAbsent': True, 'forwardRemoveExit': 0, 'reverseRuleAbsent': True,
                'reverseRemoveExit': 0, 'localListenerClosed': True, 'errors': []}
        self.assertTrue(runtime.browser_cleanup_succeeded(good))
        for key in ('sdk', 'tab', 'forwardRuleAbsent', 'reverseRuleAbsent', 'localListenerClosed', 'sdkRequired'):
            absent = copy.deepcopy(good); absent.pop(key)
            self.assertFalse(runtime.browser_cleanup_succeeded(absent), key)
        for key in ('forwardRuleAbsent', 'reverseRuleAbsent', 'localListenerClosed'):
            failed = copy.deepcopy(good); failed[key] = False
            self.assertFalse(runtime.browser_cleanup_succeeded(failed), key)
        for field in ('ok',):
            failed = copy.deepcopy(good); failed['sdk'][field] = False
            self.assertFalse(runtime.browser_cleanup_succeeded(failed))
        failed = copy.deepcopy(good); failed['sdk']['result']['cleanupSucceeded'] = False
        self.assertFalse(runtime.browser_cleanup_succeeded(failed))
        for storage in ({}, {'required': True}, {'required': True, 'ownedDatabaseClaimed': True,
                'exclusiveStorageLock': True, 'deleted': False, 'outcome': 'blocked'}):
            failed = copy.deepcopy(good); failed['sdk']['result']['storage'] = storage
            self.assertFalse(runtime.browser_cleanup_succeeded(failed))
        owned = copy.deepcopy(good)
        owned['sdk']['result']['storage'] = {'required': True, 'ownedDatabaseClaimed': True,
            'exclusiveStorageLock': True, 'deleted': True, 'outcome': 'success', 'terminal': True, 'pending': False}
        self.assertTrue(runtime.browser_cleanup_succeeded(owned))


class BuildReceiptTests(unittest.TestCase):
    def run_builder(self, *, mutate_java=False, strip_failure=False, filesystem_failure=False):
        with tempfile.TemporaryDirectory(prefix='miopunch-helper-local-') as temporary:
            root = Path(temporary)
            java = root / 'scripts/android/MioPublicShell.java'
            java.parent.mkdir(parents=True); java.write_text('initial JNI source')
            (root / 'src').mkdir(); (root / 'vendor').mkdir()
            (root / 'Cargo.toml').write_text('fake package')
            (root / 'Cargo.lock').write_text('fake lock')
            ndk = root / 'ndk'; ndk.mkdir(); (ndk / 'source.properties').write_text('fake NDK')
            d8 = root / 'd8.jar'; d8.write_bytes(b'fake d8')
            compiler = root / 'clang'; compiler.write_bytes(b'fake compiler')
            archiver = root / 'ar'; archiver.write_bytes(b'fake archiver')
            strip = root / 'strip'; strip.write_bytes(b'fake strip')
            output, target = root / 'output', root / 'target'
            def run(command, **_):
                if 'cargo' in command:
                    destination = target / 'aarch64-linux-android/debug'; destination.mkdir(parents=True)
                    (destination / 'fake-actor').write_bytes(b'\x7fELFcompiled fixture')
                    if '--lib' in command: (destination / 'libmiopunch_native_probe.so').write_bytes(b'\x7fELFcompiled lib')
                elif 'javac' in command:
                    if mutate_java: java.write_text('changed during JNI compilation')
                elif 'jar' in command:
                    (output / 'android-shell.jar').write_bytes(b'fake shell jar')
                elif str(strip) in command:
                    return subprocess.CompletedProcess(command, 37, 'controlled strip stdout', 'controlled strip stderr')
                return subprocess.CompletedProcess(command, 0, '', '')
            arguments = ['build_android_pair.py', '--ndk', str(ndk), '--output', str(output),
                         '--target-dir', str(target), '--bin', 'fake-actor']
            if mutate_java: arguments += ['--jni-shell', '--d8-jar', str(d8)]
            if strip_failure or filesystem_failure: arguments += ['--strip-deployment-debug']
            prepare = builder.prepare_deployment
            def deploy(*args):
                if filesystem_failure: raise OSError('controlled deployment filesystem failure')
                return prepare(*args)
            with patch.object(builder, 'ROOT', root), patch.object(builder, 'compiler', return_value=(compiler, archiver)), \
                 patch.object(builder, 'capture', return_value='controlled tool version'), \
                 patch.object(builder.subprocess, 'run', side_effect=run), \
                 patch.object(builder, 'prepare_deployment', side_effect=deploy), \
                 patch.object(builder.shutil, 'which', side_effect=lambda name: str(strip) if name == 'llvm-strip' else None), \
                 patch.object(sys, 'argv', arguments), redirect_stdout(io.StringIO()):
                code = builder.main()
            report = json.loads((output / 'build.json').read_text())
            before = json.loads((output / 'sources-before.json').read_text())
            after = json.loads((output / 'sources-after.json').read_text())
            return code, report, before, after

    def test_java_change_after_rust_build_invalidates_frozen_qualification(self):
        code, report, before, after = self.run_builder(mutate_java=True)
        self.assertEqual(code, 0)
        key = 'scripts/android/MioPublicShell.java'
        self.assertIn(key, before)
        self.assertNotEqual(before[key], after[key])
        self.assertFalse(report['sourceFrozenDuringBuild'])
        self.assertFalse(report['qualifiedForFrozenRun'])

    def test_failed_strip_keeps_build_and_partial_transform_receipt(self):
        code, report, _, _ = self.run_builder(strip_failure=True)
        self.assertEqual(code, 37)
        self.assertEqual(report['cargoExit'], 0)
        self.assertFalse(report['qualifiedForFrozenRun'])
        artifact = report['deployment']['fake-actor']
        self.assertEqual(artifact['transformation']['exit'], 37)
        self.assertEqual(artifact['transformation']['stdout'], 'controlled strip stdout')
        self.assertEqual(artifact['transformation']['stderr'], 'controlled strip stderr')
        self.assertTrue(artifact['compiledSha256'])
        self.assertTrue(artifact['deployedSha256'])
        self.assertTrue(report['artifacts'])

    def test_deployment_filesystem_failure_keeps_compilation_receipt(self):
        code, report, _, _ = self.run_builder(filesystem_failure=True)
        self.assertEqual(code, 1)
        self.assertEqual(report['cargoExit'], 0)
        self.assertTrue(report['artifacts'])
        self.assertIn('controlled deployment filesystem failure', report['deploymentFailure'])
        self.assertFalse(report['qualifiedForFrozenRun'])


if __name__ == '__main__':
    unittest.main()
