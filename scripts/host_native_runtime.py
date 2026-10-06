#!/usr/bin/env python3
"""Local NativeActor adapter: private JSON pipe, owned process group and directory."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import uuid

from android_pair_runtime import NativeActor, ROOT


def group_snapshot(group):
    """Only this owned group contributes identities/socket counts; unknown stays null."""
    identities, sockets, complete = set(), set(), True
    for path in Path('/proc').iterdir():
        if not path.name.isdecimal():
            continue
        try:
            raw = (path/'stat').read_text()
            fields = raw[raw.rindex(')')+2:].split()
            if int(fields[2]) != group:
                continue
            identities.add((int(path.name),int(fields[19])))
            for descriptor in (path/'fd').iterdir():
                try:
                    target = os.readlink(descriptor)
                except FileNotFoundError:
                    continue
                if target.startswith('socket:['):
                    sockets.add(target)
        except FileNotFoundError:
            continue
        except (OSError,ValueError):
            complete = False
    return identities, {'observed':complete,'processCount':len(identities) if complete else None,
        'socketCount':len(sockets) if complete else None}


def group_absent(group):
    try:
        os.killpg(group,0)
        return False
    except ProcessLookupError:
        return True
    except PermissionError:
        return None


def process_identity(pid):
    try:
        raw = (Path('/proc')/str(pid)/'stat').read_text()
        fields = raw[raw.rindex(')')+2:].split()
        return (pid,int(fields[19])),int(fields[2])
    except (OSError,ValueError):
        return None,None


def host_native_cleanup_succeeded(value, directory_required=True):
    sdk = value.get('sdkRetirement',{})
    return (value.get('forced') is False and value.get('commandExit') == 0
        and value.get('ownershipRefused') is False
        and value.get('leaderWaitedFor') is True and value.get('ownedHostProcessAbsent') is True
        and value.get('ownedHostProcessAbsentLate') is True
        and value.get('ownedProcessGroupAbsentImmediately') is True
        and value.get('ownedProcessGroupAbsentLate') is True and value.get('localReadersEnded') is True
        and value.get('localPipesClosed') is True
        and (not directory_required or value.get('ownedDirectoryRemoved') is True)
        and sdk.get('activeRtcAssociations') == 0 and sdk.get('activeRtcPumps') == 0
        and type(sdk.get('socketsBefore')) is int and type(sdk.get('socketsLate')) is int
        and sdk['socketsLate'] <= sdk['socketsBefore'])


class HostNativeActor(NativeActor):
    """Reuse NativeActor command/event API; replace only Android launch/deployment/close."""
    def __init__(self):
        self.directory = tempfile.mkdtemp(prefix='miopunch-host-')
        self._owned_directory = Path(self.directory).resolve()
        self._marker = uuid.uuid4().hex
        (self._owned_directory/'.owner').write_text(self._marker)
        self.process = None
        self.events = []
        self.condition = threading.Condition()
        self.stderr = []
        self.threads = []
        self.runs = []
        self.sequence = 0
        self.binary = None
        self.stop_command = 'stop'
        self.artifacts = {}
        self._group_identities = set()

    def deploy(self, source, name, *, executable=False, artifact=False):
        if not re.fullmatch(r'[A-Za-z0-9_.-]+',name):
            raise ValueError('owned input name must be a single safe path component')
        source = Path(source).resolve()
        destination = self._owned_directory/name
        shutil.copyfile(source,destination)
        destination.chmod(0o700 if executable else 0o600)
        expected = hashlib.sha256(source.read_bytes()).hexdigest()
        if hashlib.sha256(destination.read_bytes()).hexdigest() != expected:
            raise RuntimeError('owned deployed artifact hash does not match')
        if executable:
            self.binary = str(destination)
        if executable or artifact:
            self.artifacts[name] = expected
        return str(destination)

    def start(self, args, *, stop_command='stop', environment=None, launcher=None):
        if not self.binary and not launcher:
            raise RuntimeError('deploy an executable before native startup')
        if self.process and self.process.poll() is None:
            raise RuntimeError('an owned native process is already active')
        if any(thread.is_alive() for thread in self.threads):
            raise RuntimeError('close the prior owned native readers before restart')
        if self.process and group_absent(self.process.pid) is not True:
            raise RuntimeError('prior owned process group has not retired')
        self.events.clear()
        self.stderr.clear()
        self.stop_command = stop_command
        command = ['rtk','proxy',*(launcher or [self.binary]),*map(str,args)]
        selected_environment = dict(os.environ)
        if environment:
            selected_environment.update({str(key):str(value) for key,value in environment.items()})
        self.process = subprocess.Popen(command,cwd=ROOT,env=selected_environment,
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
        # This child has not been waited/reaped: its PID cannot yet denote a replacement.
        identity,group = process_identity(self.process.pid)
        self._group_identities = {identity} if identity is not None and group==self.process.pid else set()
        self._observe_owned_group()
        self.runs.append({'command':command,'generation':uuid.uuid4().hex,'startedMonotonic':time.monotonic()})

        def read_events():
            for line in self.process.stdout:
                if len(line) > 1024*1024:
                    with self.condition:
                        self.events.append({'event':'local-reader-failure','failure':'native event exceeded bound'})
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

        self.threads = [threading.Thread(target=reader,daemon=True) for reader in (read_events,read_errors)]
        for thread in self.threads:
            thread.start()
        return self

    def _observe_owned_group(self):
        current,snapshot = group_snapshot(self.process.pid)
        if current.intersection(self._group_identities):
            # Learn members only while an already authenticated PID+starttime remains.
            self._group_identities.update(current)
            return True,{**snapshot,'ownership':'verified'}
        if not current and group_absent(self.process.pid) is True:
            return False,{'observed':True,'processCount':0,'socketCount':0,'ownership':'retired'}
        return False,{'observed':False,'processCount':None,'socketCount':None,'ownership':'unverified'}

    def close(self, *, remove_directory=True, graceful_seconds=5):
        started = time.monotonic()
        result = {'forced':False,'nativePlatform':'host','ownershipRefused':False}
        if self.process:
            _,before = self._observe_owned_group()
            result['ownershipRefused'] = before['ownership']=='unverified'
            result['ownedProcessGroupBeforeClose'] = before
            if self.process.poll() is None:
                if self.stop_command is not None:
                    try:
                        self.send(self.stop_command)
                    except (OSError,RuntimeError):
                        result['stopWriteFailed'] = True
                try:
                    self.process.stdin.close()
                except OSError:
                    result['stdinCloseFailed'] = True
                try:
                    self.process.wait(timeout=graceful_seconds)
                except subprocess.TimeoutExpired:
                    result['naturalWaitExpired'] = True
            # Also retire owned descendants if the wrapper leader exited first.
            if group_absent(self.process.pid) is not True:
                confirmed,snapshot = self._observe_owned_group()
                result['ownershipRefused'] |= snapshot['ownership']=='unverified'
                result['ownedGroupConfirmedBeforeSignal'] = confirmed
                if confirmed:
                    result['forced'] = True
                    try:
                        os.killpg(self.process.pid,signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    term_deadline = time.monotonic()+0.3
                    try:
                        self.process.wait(timeout=0.3)
                    except subprocess.TimeoutExpired:
                        pass
                    # A wrapper exit is not proof that its child group retired.
                    while group_absent(self.process.pid) is False and time.monotonic()<term_deadline:
                        time.sleep(0.01)
                    if group_absent(self.process.pid) is not True:
                        confirmed,snapshot = self._observe_owned_group()
                        result['ownershipRefused'] |= snapshot['ownership']=='unverified'
                        if confirmed:
                            try:
                                os.killpg(self.process.pid,signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                    if self.process.poll() is None:
                        self.process.wait(timeout=1)
            result['commandExit'] = self.process.poll()
            result['leaderWaitedFor'] = self.process.returncode is not None
            result['ownedHostProcessAbsent'] = result['leaderWaitedFor']
            result['ownedProcessGroupAbsentImmediately'] = group_absent(self.process.pid)
            _,result['ownedProcessGroupImmediate'] = self._observe_owned_group()
            result['ownershipRefused'] |= result['ownedProcessGroupImmediate']['ownership']=='unverified'
            time.sleep(0.1)
            result['ownedHostProcessAbsentLate'] = self.process.poll() is not None
            result['ownedProcessGroupAbsentLate'] = group_absent(self.process.pid)
            _,result['ownedProcessGroupLate'] = self._observe_owned_group()
            result['ownershipRefused'] |= result['ownedProcessGroupLate']['ownership']=='unverified'
            result['lateDelayMs'] = 100
        else:
            result.update(ownedHostProcessAbsent=True,ownedHostProcessAbsentLate=True,
                ownedProcessGroupAbsentImmediately=True,ownedProcessGroupAbsentLate=True)
        for thread in self.threads:
            thread.join(timeout=1)
        result['localReadersEnded'] = all(not thread.is_alive() for thread in self.threads)
        if self.process and result['localReadersEnded']:
            for pipe in (self.process.stdin,self.process.stdout,self.process.stderr):
                pipe.close()
            result['localPipesClosed'] = all(pipe.closed for pipe in
                (self.process.stdin,self.process.stdout,self.process.stderr))
        if remove_directory:
            if (Path(self.directory).is_symlink() or Path(self.directory).resolve()!=self._owned_directory
                or not re.fullmatch(r'miopunch-host-[A-Za-z0-9_]+',self._owned_directory.name)
                or (self._owned_directory/'.owner').read_text()!=self._marker):
                raise RuntimeError('refusing cleanup of an unowned host directory')
            if result.get('ownedProcessGroupAbsentLate') is True:
                shutil.rmtree(self._owned_directory)
            result['ownedDirectoryRemoved'] = not self._owned_directory.exists()
        result['elapsedMs'] = round((time.monotonic()-started)*1000,3)
        if self.runs:
            self.runs[-1]['cleanup'] = result
        return result
