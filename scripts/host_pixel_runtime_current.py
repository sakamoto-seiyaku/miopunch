"""Local Host actor plus the existing Pixel page; no actor on import."""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.path.insert(0, '/home/js/Git/miopunch/scripts')
import android_pair_runtime as base
import android_pair_runtime_current as pixel

BrowserPage = base.BrowserPage
browser_cleanup_succeeded = base.browser_cleanup_succeeded


class HostDevice:
    is_host = True
    observed_model = 'LinuxHost'
    observed_transport = 'local'
    runtime_platform = 'Host/Linux'
    def __init__(self):
        self.commands = []
    def shell(self, *words, required=True, timeout=5):
        argv = ['rtk', 'proxy', *map(str, words)]
        result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        self.commands.append({'command': argv, 'exit': result.returncode})
        if required and result.returncode:
            raise RuntimeError('owned Host observation command failed')
        return result.stdout, result.returncode
    def run(self, op, source, target):
        assert op == 'push'
        assert re.fullmatch(r'/tmp/miopunch-host-pair-[0-9a-f]{32}/[A-Za-z0-9_.-]+', target)
        shutil.copy2(source, target)
        self.commands.append({'command': ['host-owned-copy', str(source), target], 'exit': 0})


class NativeActor(base.NativeActor):
    def __init__(self, device):
        self.device = device
        self.directory = '/tmp/miopunch-host-pair-' + uuid.uuid4().hex
        Path(self.directory).mkdir(mode=0o700)
        self.process, self.remote_pid, self.binary = None, None, None
        self.events, self.stderr, self.threads, self.runs = [], [], [], []
        self.condition = threading.Condition()
        self.sequence, self.stop_command, self.artifacts = 0, 'stop', {}

    def start(self, args, *, stop_command='stop', environment=None, launcher=None):
        if self.process is not None:
            if self.process.poll() is None or any(reader.is_alive() for reader in self.threads):
                raise RuntimeError('the prior owned Host SDK process/readers are still live')
            self.process = None
            self.threads = []
            self.events.clear()
            self.stderr.clear()
            self.remote_pid = None
        assert self.binary and self.process is None and not self.threads
        self.stop_command = stop_command
        argv = [*(launcher or [self.binary]), *map(str, args)]
        if environment:
            argv = ['env', *(str(k) + '=' + str(v) for k, v in environment.items()), *argv]
        shell = 'umask 077; echo $$ > ' + shlex.quote(self.directory + '/actor.pid') + '; exec ' + shlex.join(argv)
        command = ['rtk', 'proxy', 'sh', '-c', shell]
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.runs.append({'command': command, 'generation': uuid.uuid4().hex,
                          'startedMonotonic': time.monotonic(), 'platform': 'Host/Linux'})
        def read_events():
            for line in self.process.stdout:
                if len(line) > 1024 * 1024:
                    with self.condition:
                        self.events.append({'event': 'local-reader-failure', 'failure': 'native event exceeded bound'})
                        self.condition.notify_all()
                    break
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                with self.condition:
                    if len(self.events) >= 512:
                        self.events.pop(0)
                    self.events.append(value)
                    self.condition.notify_all()
            with self.condition:
                self.condition.notify_all()
        def read_errors():
            for line in self.process.stderr:
                if len(self.stderr) < 64:
                    self.stderr.append(line[:4096])
        self.threads = [threading.Thread(target=fn, daemon=True) for fn in (read_events, read_errors)]
        for reader in self.threads:
            reader.start()
        return self

    def close(self, *, remove_directory=True, graceful_seconds=5):
        result = super().close(remove_directory=False, graceful_seconds=graceful_seconds)
        assert re.fullmatch(r'/tmp/miopunch-host-pair-[0-9a-f]{32}', self.directory)
        if remove_directory and result.get('ownedRemoteProcessAbsent') is True:
            shutil.rmtree(self.directory)
            result['ownedDirectoryRemoved'] = not Path(self.directory).exists()
        else:
            result['ownedDirectoryRemoved'] = not remove_directory
        if self.process is not None:
            for pipe in (self.process.stdout, self.process.stderr):
                if pipe is not None:
                    pipe.close()
        return result


def device_for(role):
    if role == 'native':
        return HostDevice()
    if role == 'browser':
        return pixel.device_for('browser')
    raise ValueError('unknown Host/Pixel role')


def runtime_selection():
    return {'native': {'model': 'LinuxHost', 'transport': 'local', 'platform': 'Host/Linux'},
            'browser': pixel.runtime_selection()['browser']}
