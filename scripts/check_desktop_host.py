#!/usr/bin/env python3
"""Real Linux Tauri/WebKit window -> IPC -> SDK -> independent file observer.

Runs an owned Ubuntu toolchain/runtime container on the current WSLg X11/DBus
session. Its network has only loopback, and its home/autostart entries are private.
This qualifies a real window and silent launch, not an actual login trigger.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid

from check_forwarding import Node, isolated_environment
from forwarding_local_resources import configure_local_resources
from web_process_owner import OrdinaryChild, register_process

ROOT = Path(__file__).resolve().parents[1]
IMAGE = "sha256:5bf4699470632ddf19a61a5e4d21795e30ffb3ef8a0f621d90137d27eedda942"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def until(predicate, label, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError(label)


class Member(Node):
    def __init__(self, runner, invitation):
        self.runner, self.label = runner, "member"
        self.directory = str(runner.private / "member")
        self.stderr = (runner.output / "member.stderr").open("w")
        self.process = subprocess.Popen([str(runner.binary), "join", self.directory, str(invitation), str(runner.native_configuration)],
            env=isolated_environment(), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, text=True)
        self.queue, self.reader_error, self.owner = queue.Queue(maxsize=128), None, None
        self.reader = threading.Thread(target=self.read, daemon=True)
        runner.nodes.append(self)
        self.owner = register_process(self.process, runner.output / "member-owner.jsonl")
        self.reader.start()
        ready = self.receive()
        assert ready["event"] == "ready"
        self.identity, self.address = ready["status"]["node"], ready["address"]
        assert self.call("request", path=str(invitation))["stage"] == "Complete"

    def close(self):
        if hasattr(self, "cleanup"):
            return self.cleanup
        result = super().close()
        if self.owner:
            self.owner.close()
        return result


class Desktop:
    def __init__(self, runner, label, background=False):
        self.runner, self.label = runner, label
        self.verification = runner.private / label
        self.verification.mkdir(mode=0o700)
        self.sequence, self.quit_requested = 0, False
        self.owner = None
        self.logs = [(runner.output / (label + "." + suffix)).open("w") for suffix in ["stdout", "stderr"]]
        command = [str(runner.desktop), "--data-dir", str(runner.private / "node"), "--verification-dir", str(self.verification)]
        if background:
            command.append("--background")
        self.process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=self.logs[0], stderr=self.logs[1])
        runner.desktops.append(self)
        self.owner = register_process(self.process, runner.output / (label + "-owner.jsonl"))

    def events(self):
        path = self.verification / "events.jsonl"
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text().splitlines() if line]

    def event(self, name, count=1):
        def observed():
            assert self.process.poll() is None, "desktop exited before " + name
            values = [e for e in self.events() if e["event"] == name]
            return values if len(values) >= count else None
        return until(observed, "desktop event missing: " + name)

    def call(self, action, **fields):
        self.sequence += 1
        identifier = self.sequence
        request = {"id":identifier,"action":action,**fields}
        temporary = self.verification / "request.next"
        temporary.write_text(json.dumps(request))
        temporary.replace(self.verification / "request.json")
        path = self.verification / ("result-%d.json" % identifier)
        def completed():
            if path.exists():
                return json.loads(path.read_text())
            assert self.process.poll() is None, "desktop exited before IPC result"
        result = until(completed, "WebView IPC response missing: " + action)
        self.runner.record(self.label, action, result)
        assert result["ok"], result
        if action == "quit":
            assert result["stopped"]
            self.quit_requested = True
        return result.get("result", result)

    def request(self, operation, **fields):
        result = self.call("request", request={"id":self.sequence+1,"op":operation,**fields})
        assert result["ok"], operation
        return result["result"]

    def close(self):
        if hasattr(self, "cleanup"):
            return self.cleanup
        forced = False
        if self.process.poll() is None and not self.quit_requested:
            try:
                self.runner.secondary(self)
                self.call("quit")
            except BaseException as failure:
                self.runner.record(self.label, "normal_quit_failed", {"type":type(failure).__name__,"message":str(failure)})
                # Original qualified Popen only. Forced settlement never passes.
                if self.owner is None:
                    raise RuntimeError("desktop launch unqualified; no signal sent")
                self.owner.send(signal.SIGTERM)
                forced = True
        self.process.wait(timeout=20)
        if self.owner:
            self.owner.close()
        for stream in self.logs:
            stream.close()
        self.cleanup = {"desktop":self.label,"exit":self.process.returncode,"forced":forced,"normal_quit":self.quit_requested}
        assert self.process.returncode == 0 and not forced and self.quit_requested, self.cleanup
        return self.cleanup


class Runner:
    def __init__(self, args):
        self.args, self.output, self.private = args, args.output, args.private
        self.binary = Path(json.loads(args.sdk_receipt.read_text())["pin"])
        desktop = json.loads(args.desktop_receipt.read_text())
        self.desktop = Path(desktop["pin"])
        assert desktop["status"] == "pass" and sha(self.desktop) == desktop["sha256"]
        sdk = json.loads(args.sdk_receipt.read_text())
        assert sdk["exit"] == 0 and sha(self.binary) == sdk["sha256"]
        self.nodes, self.desktops, self.sequence = [], [], 0
        assert Path.home() == Path('/home/ubuntu') and Path.home().samefile(self.private / 'home')
        assert Path.home().stat().st_uid == os.getuid() and Path.home().stat().st_mode & 0o777 == 0o700
        assert [name for _,name in socket.if_nameindex()] == ['lo']
        self.controller_sources = {}
        for name in [__file__, ROOT / "scripts/check_forwarding.py", ROOT / "scripts/web_process_owner.py"]:
            self.freeze_source(Path(name))

    def freeze_source(self, path):
        self.controller_sources[str(path)] = sha(path)
        destination = self.output / "controller-source" / path.name
        destination.parent.mkdir(exist_ok=True)
        destination.write_bytes(path.read_bytes())

    def record(self, node, operation, result):
        if operation == "local_resource_allow_list":
            result = dict(result, desktop_context_removed="SDK helpers only; Tauri retains the assigned X11/DBus session")
        with (self.output / "events.jsonl").open("a") as stream:
            stream.write(json.dumps({"node":node,"operation":operation,"result":result,"monotonic":time.monotonic()}) + "\n")

    def windows(self, present):
        since = time.monotonic()
        def observed():
            path = self.output / "x11-current.json"
            if not path.exists():
                return None
            state = json.loads(path.read_text())
            assert not state.get("error"), state
            return state if state["monotonic"] >= since and bool(state["windows"]) == present else None
        return until(observed, "independent X11 window state did not change")

    def secondary(self, desktop):
        receipt = self.output / ("secondary-%s-%d-owner.jsonl" % (desktop.label, desktop.sequence))
        process = subprocess.Popen([str(self.desktop), "--data-dir", str(self.private / "node")],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        child = OrdinaryChild(process, receipt)
        self.nodes.append(child)
        try:
            try:
                child.owner = register_process(process, receipt)
            except RuntimeError:
                if process.poll() is None:
                    raise
            stdout, stderr = child.communicate(timeout=10)
            assert process.returncode == 0, stderr
        finally:
            child.close()

    def scenario(self):
        configure_local_resources(self, self.args, owner_factory=register_process)
        resources = json.loads(self.native_configuration.read_text())
        assert not list((Path.home()/'.config/autostart').glob('miopunch.desktop'))
        initial = Desktop(self, "initial")
        initial.event("webview_ready")
        first_window = self.windows(True)
        assert initial.call("state")["autostart"] is True
        autostart = Path.home()/'.config/autostart/miopunch.desktop'
        entry = autostart.read_text()
        assert str(self.desktop) in entry and "--background" in entry and str(self.private / "node") in entry
        ready = initial.call("start", mode={"action":"create"}, resources=resources)
        assert ready["status"]["stage"] == "AwaitingRecoverySave"
        identity, address = ready["status"]["node"], ready["address"]
        initial.request("export_recovery_card", path=str(self.private / "card"))
        initial.request("acknowledge_recovery_saved")
        invitation = self.private / "invite"
        initial.request("issue_precode", path=str(invitation), seconds=300, max_uses=1)
        member = Member(self, invitation)
        assert member.identity != identity
        assert initial.request("ping", address=member.address)["reply"] == "PONG"
        destination = self.private / "received"
        initial.request("files_enable", directory=str(destination))
        before = initial.call("state")
        initial.call("close")
        initial.event("window_destroyed")
        self.windows(False)
        assert initial.process.poll() is None
        assert member.call("ping", address=address)["reply"] == "PONG"
        payload = bytes(i % 239 for i in range(65539))
        path = self.private / "actual.bin"
        path.write_bytes(payload)
        task = member.call("file_send", address=address, path=str(path))["task"]
        until(lambda: any(t["id"] == task and t["state"] == "complete" for t in member.call("files_status")["tasks"]), "background file not complete")
        files = list(destination.iterdir())
        assert len(files) == 1 and files[0].read_bytes() == payload
        self.record("observer", "saved_while_window_absent", {"bytes":len(payload),"sha256":sha(files[0]),"desktop_pid":initial.process.pid})
        self.secondary(initial)
        initial.event("webview_ready", 2)
        second_window = self.windows(True)
        after = initial.call("state")
        assert after["pid"] == before["pid"] and after["node"]["result"]["node"] == identity
        assert after["windows_created"] == 2 and after["windows_ready"] == 2 and after["windows"] == ["main"]
        assert first_window["windows"] != second_window["windows"]
        assert initial.call("autostart", enabled=False) is False
        assert not autostart.exists()
        initial.call("quit")
        initial.close()
        self.windows(False)
        silent = Desktop(self, "silent", background=True)
        opened = silent.event("node_opened")
        assert opened[-1]["detail"]["ok"] is True
        assert not any(e["event"] == "window_created" for e in silent.events())
        self.windows(False)
        assert not autostart.exists()
        self.secondary(silent)
        silent.event("webview_ready")
        self.windows(True)
        state = silent.call("state")
        assert state["node"]["result"]["node"] == identity and state["node"]["result"]["stage"] == "Complete"
        assert state["autostart"] is False
        assert silent.request("ping", address=member.address)["reply"] == "PONG"
        assert silent.call("autostart", enabled=True) is True and autostart.exists()
        assert "--background" in autostart.read_text()
        assert silent.call("autostart", enabled=False) is False and not autostart.exists()
        member.close()
        silent.call("quit")
        silent.close()
        self.windows(False)
        self.record("observer", "qualified", {"real_webview_ipc":True,"same_process_reopen":True,"same_identity_restart":True,
            "default_autostart_on":True,"autostart_off_persisted":True,"silent_launch":True,"actual_login_trigger":False,
            "independent_saved_bytes":len(payload),"clipboard_tested":False})

    def run(self):
        report = {"status":"failed","clock_start":time.monotonic(),"runtime":"Ubuntu24.04 container on actual WSLg X11 session","image":IMAGE}
        error = None
        try:
            self.scenario()
            report["status"] = "pass"
        except BaseException as failure:
            error = failure
            report.update(failure_type=type(failure).__name__,failure=str(failure))
        cleanup, errors = [], []
        for owner in list(reversed(self.desktops)) + list(reversed(self.nodes)):
            try:
                cleanup.append(owner.close())
            except BaseException as failure:
                errors.append({"type":type(failure).__name__,"message":str(failure)})
        for desktop in self.desktops:
            source = desktop.verification / "events.jsonl"
            if source.exists():
                (self.output / (desktop.label + "-native-events.jsonl")).write_bytes(source.read_bytes())
        report.update(cleanup=cleanup,cleanup_errors=errors,clock_end=time.monotonic(),
            controller_drift=[p for p,h in self.controller_sources.items() if sha(p)!=h],
            memory_peak=Path('/sys/fs/cgroup/memory.peak').read_text().strip())
        if errors or report["controller_drift"]:
            report["status"] = "failed"
        (self.output / "result.json").write_text(json.dumps(report,indent=2)+'\n')
        if error:
            raise error
        assert report["status"] == "pass", report


def outer(args):
    for path in [args.desktop_receipt, args.sdk_receipt, args.relay_receipt, args.relay_binary]:
        assert path.is_file(), "runtime input is absent: " + str(path)
    args.output.mkdir(parents=True, exist_ok=False)
    private = Path(tempfile.mkdtemp(prefix="m121-desktop-"))
    for name in ["home","runtime","cache","config","data"]:
        (private/name).mkdir(mode=0o700)
    args.private = private
    stop = threading.Event()
    observations = []
    def observe():
        while not stop.is_set():
            result = subprocess.run(['rtk','proxy','xwininfo','-root','-tree'],capture_output=True,text=True)
            row = {"monotonic":time.monotonic(),"windows":re.findall(r'(0x[0-9a-fA-F]+) "miopunch host"',result.stdout)}
            if result.returncode:
                row["error"] = result.stderr
            observations.append(row)
            temporary = args.output/'x11-current.next'
            temporary.write_text(json.dumps(row));temporary.replace(args.output/'x11-current.json')
            stop.wait(.1)
    observer = threading.Thread(target=observe)
    observer.start()
    until(lambda:(args.output/'x11-current.json').exists(), 'X11 observer absent')
    baseline = json.loads((args.output/'x11-current.json').read_text())
    assert not baseline.get('error') and not baseline['windows'], 'preexisting matching desktop window'
    command = ['rtk','proxy','docker','run','--rm','--cidfile',str(args.output/'runtime.cid'),'--network','none','--memory','2g',
        '--user',f'{os.getuid()}:{os.getgid()}','--entrypoint','/usr/bin/python3',
        '-v',f'{ROOT}:{ROOT}:ro','-v',f'{private}:{private}','-v',f'{private}/home:/home/ubuntu',
        '-v',f'{args.sdk_receipt}:{args.sdk_receipt}:ro','-v',f'{args.desktop_receipt}:{args.desktop_receipt}:ro',
        '-v',f'{args.output}:{args.output}','-v','/tmp/.X11-unix:/tmp/.X11-unix:ro',
        '-v','/run/user/1000/bus:/run/user/1000/bus','-w',str(ROOT),
        '-e','DISPLAY=:0','-e','GDK_BACKEND=x11','-e','DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus',
        '-e',f'XDG_RUNTIME_DIR={private}/runtime','-e',f'XDG_CACHE_HOME={private}/cache',
        '-e',f'XDG_CONFIG_HOME={private}/config','-e',f'XDG_DATA_HOME={private}/data',
        IMAGE,'-B',str(Path(__file__).resolve()),'--inside','--private',str(private),'--output',str(args.output),
        '--desktop-receipt',str(args.desktop_receipt),'--sdk-receipt',str(args.sdk_receipt),
        '--relay-binary',str(args.relay_binary),'--relay-receipt',str(args.relay_receipt)]
    (args.output/'launch.json').write_text(json.dumps({'command':command,'private':str(private),'source_sha256':sha(__file__),
        'private_budget_bytes':256*1024**2,'raw_budget_bytes':16*1024**2,'memory_limit_bytes':2*1024**3},indent=2)+'\n')
    try:
        with (args.output/'controller.stdout').open('w') as stdout,(args.output/'controller.stderr').open('w') as stderr:
            result = subprocess.run(command,stdout=stdout,stderr=stderr)
    finally:
        stop.set();observer.join(timeout=5)
        (args.output/'x11-observations.json').write_text(json.dumps(observations,indent=2)+'\n')
    report={'exit':result.returncode,'observer_terminal':not observer.is_alive(),'private':str(private),'container_cid':(args.output/'runtime.cid').read_text().strip()}
    (args.output/'outer-result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
    assert result.returncode==0, 'desktop controller failed; original evidence retained'


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inside',action='store_true')
    parser.add_argument('--private',type=Path)
    for name in ['output','desktop-receipt','sdk-receipt','relay-binary','relay-receipt']:
        parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    if args.inside:
        Runner(args).run()
    else:
        outer(args)
