#!/usr/bin/env python3
"""M75 owned page substitution and thin Host Chromium lifecycle adapter."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid

from android_pair_runtime import BrowserPage, ROOT, browser_cleanup_succeeded
from host_native_runtime import group_snapshot, group_absent, process_identity
from check_browser_relay_capability import OwnedCdpObserver

PAGE = ROOT / 'browser-probe/harness/mio75-browser-pair-page.js'
SIDECAR = ROOT / 'browser-probe/harness/mio75-host-browser.mjs'


class PairPage(BrowserPage):
    def __init__(self, device, package, *, prelude=""):
        super().__init__(device, package, prelude=prelude, environment_rtc_probe=False)
        original = self.server.RequestHandlerClass

        class Handler(original):
            def do_GET(self):
                if self.path == '/android-pair-page.js':
                    return self.send_bytes(PAGE.read_bytes(), 'text/javascript; charset=utf-8')
                return super().do_GET()

        self.server.RequestHandlerClass = Handler


class HostBrowserPage(PairPage):
    def __init__(self, package, executable, *, prelude=""):
        self.executable = str(Path(executable).resolve())
        self.directory = Path(tempfile.mkdtemp(prefix='mio75-host-browser-')).resolve()
        self.marker = uuid.uuid4().hex
        try:
            (self.directory / '.owner').write_text(self.marker)
            super().__init__(None, package, prelude=prelude)
        except Exception:
            shutil.rmtree(self.directory)
            raise
        self.process = None
        self.identities = set()
        self.sidecar_events = []
        self.sidecar_condition = threading.Condition()
        self.readers = []
        self.sidecar_errors = []  # Never publish arbitrary browser stderr.

    def _observe_group(self):
        if self.process is None:
            return {'observed': True, 'processCount': 0, 'socketCount': 0, 'ownership': 'not-started'}
        current, facts = group_snapshot(self.process.pid)
        if current.intersection(self.identities):
            self.identities.update(current)
            return {**facts, 'ownership': 'verified'}
        if not current and group_absent(self.process.pid) is True:
            return {**facts, 'ownership': 'retired'}
        return {'observed': False, 'processCount': None, 'socketCount': None, 'ownership': 'unverified'}

    def start(self, timeout=25):
        if self.process is not None:
            raise RuntimeError('owned Host Browser sidecar cannot be restarted')
        self.process = subprocess.Popen(['rtk', 'proxy', 'node', str(SIDECAR), self.executable,
            str(self.directory / 'profile'), self.url], cwd=ROOT, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        identity, group = process_identity(self.process.pid)
        if identity is not None and group == self.process.pid:
            self.identities.add(identity)
        self.opened = True

        def read_events():
            for line in self.process.stdout:
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                with self.sidecar_condition:
                    if len(self.sidecar_events) < 512:
                        self.sidecar_events.append(value)
                    self.sidecar_condition.notify_all()

        def read_errors():
            for line in self.process.stderr:
                if len(self.sidecar_errors) < 64:
                    self.sidecar_errors.append(line[:4096])

        self.readers = [threading.Thread(target=reader, daemon=True) for reader in (read_events, read_errors)]
        for reader in self.readers:
            reader.start()
        deadline = time.monotonic() + timeout
        while self.terminal is None and time.monotonic() < deadline:
            if self.ready is not None and any(event.get('event') == 'ready' for event in self.sidecar_events):
                break
            self._observe_group()
            with self.condition:
                self.condition.wait(timeout=min(0.1, max(0, deadline-time.monotonic())))
            if self.process.poll() is not None:
                break
        ready = next((event for event in self.sidecar_events if event.get('event') == 'ready'), {})
        self.browser_version = ready.get('browserVersion')
        if self.ready is None or self.terminal or not ready.get('normalPublicTrust'):
            raise RuntimeError('owned Host Browser page setup failed; no SDK qualification')
        return self.ready

    def receipt(self):
        with self.sidecar_condition:
            return {'available': self.browser_version is not None,
                'scope': 'existing owned Host page CDP Network metadata only',
                'events': [event['observation'] for event in self.sidecar_events
                           if event.get('event') == 'network-observation'],
                'framesOrHeadersRecorded': False}

    def _close_owned_tab(self):
        if self.process is None:
            return {'ownedTargetAbsent': True, 'forced': False, 'contextRequired': False}
        before = self._observe_group()
        forced, ownership_refused = False, False
        if self.process.poll() is None:
            try:
                self.process.stdin.write('close\n'); self.process.stdin.flush()
            except (BrokenPipeError, OSError):
                pass
        deadline = time.monotonic()+5
        while time.monotonic() < deadline:
            self._observe_group()
            if self.process.poll() is not None and group_absent(self.process.pid) is True:
                break
            time.sleep(0.02)
        if group_absent(self.process.pid) is not True:
            forced = True
            # Numeric process groups cannot be adopted after their verified anchor exits.
            # Signal only exact identities authenticated while an owned anchor lived.
            self._observe_group()
            for pid, starttime in self.identities:
                actual, group = process_identity(pid)
                if actual == (pid, starttime) and group == self.process.pid:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            ownership_refused = group_absent(self.process.pid) is not True
        try:
            command_exit = self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            command_exit = None
        immediate = self._observe_group()
        absent_immediate = group_absent(self.process.pid)
        time.sleep(0.1)
        late = self._observe_group()
        for reader in self.readers:
            reader.join(timeout=1)
        readers_ended = all(not reader.is_alive() for reader in self.readers)
        if readers_ended:
            for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
                stream.close()
        ended = next((event for event in self.sidecar_events if event.get('event') == 'closed'), {})
        return {'ownedTargetAbsent': ended.get('ownedTargetAbsent') is True,
            'contextClosed': ended.get('contextClosed') is True, 'forced': forced,
            'ownershipRefused': ownership_refused, 'commandExit': command_exit,
            'localReadersEnded': readers_ended,
            'localPipesClosed': all(stream.closed for stream in (self.process.stdin, self.process.stdout, self.process.stderr)),
            'before': before, 'immediate': immediate, 'late': late,
            'processGroupAbsentImmediate': absent_immediate is True,
            'processGroupAbsentLate': late.get('ownership') == 'retired',
            'observations': ended.get('observations', [])}

    def close(self):
        if self.closed:
            return self.cleanup
        result = super().close()
        tab = result.get('tab', {})
        safe = (not tab.get('forced') and tab.get('commandExit') == 0
            and tab.get('ownershipRefused') is False and tab.get('contextClosed') is True
            and tab.get('processGroupAbsentImmediate') is True and tab.get('processGroupAbsentLate') is True
            and tab.get('localReadersEnded') is True and tab.get('localPipesClosed') is True)
        if self.process is None:
            safe = True
        marker = self.directory / '.owner'
        if safe and not self.directory.is_symlink() and marker.is_file() and marker.read_text() == self.marker:
            shutil.rmtree(self.directory)
        result['ownedProfileRemoved'] = not self.directory.exists()
        result['cleanupSucceeded'] = browser_cleanup_succeeded(result) and safe and result['ownedProfileRemoved']
        return result


class PixelCdpObserver(OwnedCdpObserver):
    """Existing exact-tab observer with PID/starttime-checked process retirement."""
    def __init__(self, page):
        super().__init__(page)
        self.identities = set()

    def start(self):
        result = super().start()
        if self.process is not None and self.process.poll() is None:
            identity, group = process_identity(self.process.pid)
            if identity is not None and group == self.process.pid:
                self.identities.add(identity)
                current, _ = group_snapshot(group)
                if current.intersection(self.identities):
                    self.identities.update(current)
        return result

    def close(self):
        started = time.monotonic()
        result = {'forced': False, 'ownershipRefused': False}
        if self.process is not None:
            current, _ = group_snapshot(self.process.pid)
            if current.intersection(self.identities):
                self.identities.update(current)
            if self.process.poll() is None:
                try:
                    self.process.stdin.write('stop\n'); self.process.stdin.flush()
                    self.process.stdin.close()
                except (OSError, ValueError):
                    result['stopWriteFailed'] = True
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    result['forced'] = True
            if group_absent(self.process.pid) is not True:
                result['forced'] = True
                for pid, starttime in self.identities:
                    actual, group = process_identity(pid)
                    if actual == (pid, starttime) and group == self.process.pid:
                        try:
                            os.kill(pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                try:
                    self.process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    result['ownershipRefused'] = True
            result['commandExit'] = self.process.poll()
            result['processGroupAbsentImmediate'] = group_absent(self.process.pid)
            time.sleep(0.1)
            result['processGroupAbsentLate'] = group_absent(self.process.pid)
        else:
            result.update(commandExit=0, processGroupAbsentImmediate=True, processGroupAbsentLate=True)
        if self.reader:
            self.reader.join(timeout=1)
        result['readerEnded'] = not self.reader or not self.reader.is_alive()
        if self.process is not None and result['readerEnded']:
            for pipe in (self.process.stdin, self.process.stdout, self.process.stderr):
                if pipe is not None:
                    pipe.close()
        result['elapsedMs'] = round((time.monotonic()-started)*1000, 3)
        return result
