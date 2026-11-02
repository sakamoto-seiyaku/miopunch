#!/usr/bin/env python3
"""Real LightDM/PAM -> LXSession XDG autostart -> existing Tauri/SDK pin.

Uses a fresh, private Linux container/home/display. Never changes the host's
desktop, login manager or autostart files. The SDK is not compiled here.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import pwd
import queue
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import uuid

from check_desktop_host import Desktop, until
from check_forwarding import Node, isolated_environment
from web_process_owner import register_process

ROOT=Path(__file__).resolve().parents[1]


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def case_paths(case):
    if case=='spaces':return 'app files','node state'
    if case=='quoting':
        # Space, quote, dollar, backtick, percent and backslash are literal
        # Linux path characters, not shell or desktop-launcher substitutions.
        special=''.join(chr(code) for code in [32,34,39,36,96,37,92,32])
        return 'app'+special+'files','node'+special+'state'
    return 'appfiles','node'


class OwnedNode(Node):
    """Native CLI protocol with ownership registered before startup/readiness."""
    def __init__(self,runner,label,action,invitation):
        self.runner,self.label=runner,label
        self.directory=str(runner.private/label)
        self.stderr=(runner.output/(label+'.stderr')).open('w')
        self.process=subprocess.Popen([str(runner.binary),action,self.directory,str(invitation),str(runner.native_configuration)],
            env=isolated_environment(),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.stderr,text=True)
        self.queue=queue.Queue(maxsize=128);self.reader_error=None;self.owner=None
        self.reader=threading.Thread(target=self.read,daemon=True)
        runner.nodes.append(self)
        self.owner=register_process(self.process,runner.output/(label+'-owner.jsonl'))
        self.reader.start()
        ready=self.receive();assert ready['event']=='ready'
        self.identity,self.address=ready['status']['node'],ready['address']

    def close(self):
        if hasattr(self,'cleanup'):return self.cleanup
        result=super().close()
        self.owner.close()
        return result


class Service:
    def __init__(self, runner, label, command, **fields):
        self.runner,self.label=runner,label
        self.files=[(runner.output/(label+'.'+suffix)).open('w') for suffix in ['stdout','stderr']]
        self.process=subprocess.Popen(command,stdout=self.files[0],stderr=self.files[1],**fields)
        self.owner=None
        runner.services.append(self)
        self.owner=register_process(self.process,runner.output/(label+'-owner.jsonl'))
        runner.record(label,'launched',dict(pid=self.process.pid,command=command))

    def close(self):
        if hasattr(self,'cleanup'):return self.cleanup
        if self.process.poll() is None:
            # Qualified original handle of a disposable display/session service.
            self.owner.send(signal.SIGTERM)
        code=self.process.wait(timeout=20)
        self.owner.close()
        for stream in self.files:stream.close()
        self.cleanup=dict(service=self.label,exit=code,original_pid=self.process.pid)
        return self.cleanup


class XImage(ctypes.Structure):
    _fields_=[('width',ctypes.c_int),('height',ctypes.c_int),('xoffset',ctypes.c_int),('format',ctypes.c_int),
        ('data',ctypes.c_void_p),('byte_order',ctypes.c_int),('bitmap_unit',ctypes.c_int),('bitmap_bit_order',ctypes.c_int),
        ('bitmap_pad',ctypes.c_int),('depth',ctypes.c_int),('bytes_per_line',ctypes.c_int),('bits_per_pixel',ctypes.c_int),
        ('red_mask',ctypes.c_ulong),('green_mask',ctypes.c_ulong),('blue_mask',ctypes.c_ulong)]


class Display:
    def __init__(self):
        self.x=ctypes.CDLL('libX11.so.6')
        self.xt=ctypes.CDLL('libXtst.so.6')
        self.x.XOpenDisplay.argtypes=[ctypes.c_char_p];self.x.XOpenDisplay.restype=ctypes.c_void_p
        self.handle=self.x.XOpenDisplay(b':90');assert self.handle
        self.x.XDefaultRootWindow.argtypes=[ctypes.c_void_p];self.x.XDefaultRootWindow.restype=ctypes.c_ulong
        self.root=self.x.XDefaultRootWindow(self.handle)
        self.x.XQueryTree.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),ctypes.POINTER(ctypes.c_uint)]
        self.x.XFetchName.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.POINTER(ctypes.c_void_p)]
        self.x.XFree.argtypes=[ctypes.c_void_p]
        self.x.XTranslateCoordinates.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_int,ctypes.c_int,ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_ulong)]
        self.x.XFlush.argtypes=[ctypes.c_void_p]
        self.xt.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
        self.xt.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_bool,ctypes.c_ulong]
        self.x.XGetImage.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_int,ctypes.c_uint,ctypes.c_uint,ctypes.c_ulong,ctypes.c_int]
        self.x.XGetImage.restype=ctypes.POINTER(XImage)
        self.x.XDestroyImage.argtypes=[ctypes.POINTER(XImage)]

    def checkbox_rendered_on(self,window):
        # The pinned runtime's actual Adwaita checkbox must be painted checked
        # before clicking. A native window can exist before its WebView loads.
        pointer=self.x.XGetImage(self.handle,window,110,45,40,35,0xffffffffffffffff,2)
        if not pointer:return False
        try:
            value=pointer.contents
            assert value.bits_per_pixel==32 and value.byte_order==0
            assert (value.red_mask,value.green_mask,value.blue_mask)==(0xff0000,0xff00,0xff)
            data=ctypes.string_at(value.data,value.bytes_per_line*value.height)
            blue=sum(data[i]>160 and data[i+1]>80 and data[i+2]<100 and data[i]>data[i+2]+80 for i in range(0,len(data),4))
            return blue>=10
        finally:self.x.XDestroyImage(pointer)

    def windows(self):
        found=[]
        def visit(window,depth):
            name=ctypes.c_void_p()
            if self.x.XFetchName(self.handle,window,ctypes.byref(name)) and name.value:
                text=ctypes.string_at(name.value).decode(errors='replace');self.x.XFree(name)
                if text=='miopunch host':found.append(window)
            if depth>3:return
            root,parent=ctypes.c_ulong(),ctypes.c_ulong();children=ctypes.POINTER(ctypes.c_ulong)();count=ctypes.c_uint()
            if self.x.XQueryTree(self.handle,window,ctypes.byref(root),ctypes.byref(parent),ctypes.byref(children),ctypes.byref(count)):
                try:
                    for index in range(count.value):visit(children[index],depth+1)
                finally:
                    if children:self.x.XFree(children)
        visit(self.root,0)
        return found

    def click(self,window,x,y):
        actual_x,actual_y=ctypes.c_int(),ctypes.c_int();child=ctypes.c_ulong()
        assert self.x.XTranslateCoordinates(self.handle,window,self.root,x,y,ctypes.byref(actual_x),ctypes.byref(actual_y),ctypes.byref(child))
        assert self.xt.XTestFakeMotionEvent(self.handle,-1,actual_x.value,actual_y.value,0)
        assert self.xt.XTestFakeButtonEvent(self.handle,1,True,0)
        assert self.xt.XTestFakeButtonEvent(self.handle,1,False,0)
        self.x.XFlush(self.handle)


class Runner:
    def __init__(self,args):
        self.args,self.output,self.private=args,args.output,args.private
        self.output.mkdir(exist_ok=False)
        os.chown(self.output,1000,1000)
        self.binary=Path(json.loads(args.sdk_receipt.read_text())['pin'])
        desktop=json.loads(args.desktop_receipt.read_text())
        assert desktop['exit']==0 and sha(desktop['pin'])==desktop['sha256']
        sdk=json.loads(args.sdk_receipt.read_text());assert sdk['exit']==0 and sha(self.binary)==sdk['sha256']
        self.case=args.case
        self.nodes,self.desktops,self.services,self.sequence=[],[],[],0
        self.controller_sources={}
        self.freeze_source(Path(__file__))
        self.freeze_source(ROOT/'scripts/check_desktop_host.py')
        self.freeze_source(ROOT/'scripts/check_forwarding.py')
        self.freeze_source(ROOT/'scripts/web_process_owner.py')
        self.user=pwd.getpwnam('ubuntu');assert self.user.pw_uid==1000 and self.user.pw_dir=='/home/ubuntu'
        self.home=Path('/home/ubuntu');assert self.home.samefile(self.private/'home')
        self.home.mkdir(exist_ok=True);os.chown(self.home,1000,1000);self.home.chmod(0o700)
        self.config_home=self.private/'xdg config' if self.case=='xdg' else self.home/'.config'
        self.config_home.mkdir(mode=0o700,exist_ok=True);os.chown(self.config_home,1000,1000)
        self.entry=self.config_home/'autostart/miopunch.desktop'
        self.runtime=self.private/'runtime';self.runtime.mkdir(mode=0o700);os.chown(self.runtime,1000,1000)
        self.native_configuration=self.private/'resources.json'
        self.native_configuration.write_text(json.dumps(dict(defaults=dict(revision=2,resources=[]),replacement=[],disabled=[],
            disabled_uses=['Observation','Lookup','Signaling','DataRelay'],lan_discovery='Disabled',port_mapping='Disabled')))
        os.chown(self.native_configuration,1000,1000)
        app_directory,node_directory=case_paths(self.case)
        self.desktop=self.private/app_directory/'miopunch-desktop'
        assert self.desktop.is_file() and sha(self.desktop)==desktop['sha256']
        self.node_directory=self.private/node_directory

    def freeze_source(self,path):
        path=Path(path);self.controller_sources[str(path)]=sha(path)
        target=self.output/'controller-source'/path.name;target.parent.mkdir(exist_ok=True);target.write_bytes(path.read_bytes())

    def record(self,node,operation,result):
        with (self.output/'events.jsonl').open('a') as stream:
            stream.write(json.dumps(dict(node=node,operation=operation,result=result,monotonic=time.monotonic()))+'\n')

    def secondary(self,desktop):
        if desktop.verification.is_dir() and self.display.windows():return
        raise RuntimeError('unexpected verification-window recovery; original failure retained')

    def app_processes(self):
        result=[]
        for proc in Path('/proc').iterdir():
            if not proc.name.isdigit():continue
            try:
                args=[x.decode() for x in (proc/'cmdline').read_bytes().split(b'\0') if x]
                if not args or args[0]!=str(self.desktop):continue
                fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
                result.append(dict(pid=int(proc.name),parent=int(fields[1]),start_ticks=fields[19],args=args))
            except (OSError,UnicodeError):continue
        return result

    def service(self,label,command,**fields):
        return Service(self,label,command,**fields)

    def gui_environment(self,bus):
        env=os.environ.copy()
        for key in list(env):
            if key.lower().endswith('_proxy'):env.pop(key)
        env.update(HOME=str(self.home),USER='ubuntu',LOGNAME='ubuntu',DISPLAY=':90',GDK_BACKEND='x11',
            XDG_RUNTIME_DIR=str(self.runtime),DBUS_SESSION_BUS_ADDRESS=bus,XDG_CONFIG_HOME=str(self.config_home))
        return env

    def initial(self,bus):
        # Reuse the real WebView IPC driver, selecting the same directory that
        # will be opened by the unmodified product autostart registration.
        verification=self.private/'initial';verification.mkdir(mode=0o700);os.chown(verification,1000,1000)
        obj=Desktop.__new__(Desktop);obj.runner=self;obj.label='initial';obj.verification=verification
        obj.sequence=0;obj.quit_requested=False;obj.owner=None
        obj.logs=[(self.output/('initial.'+s)).open('w') for s in ['stdout','stderr']]
        obj.process=subprocess.Popen([str(self.desktop),'--data-dir',str(self.node_directory),'--verification-dir',str(verification)],
            stdin=subprocess.DEVNULL,stdout=obj.logs[0],stderr=obj.logs[1],env=self.gui_environment(bus),user=1000,group=1000)
        self.desktops.append(obj);obj.owner=register_process(obj.process,self.output/'initial-owner.jsonl')
        obj.event('webview_ready')
        resources=json.loads(self.native_configuration.read_text())
        ready=obj.call('start',mode=dict(action='create'),resources=resources)
        assert ready['status']['stage']=='AwaitingRecoverySave'
        self.identity=ready['status']['node'];self.original_address=ready['address']
        obj.request('export_recovery_card',path=str(self.private/'card'))
        obj.request('acknowledge_recovery_saved')
        invitation=self.private/'invitation'
        obj.request('issue_precode',path=str(invitation),seconds=600,max_uses=1)
        peer=OwnedNode(self,'peer','join',invitation)
        assert peer.identity!=self.identity and peer.call('request',path=str(invitation))['stage']=='Complete'
        assert obj.request('ping',address=peer.address)['reply']=='PONG'
        self.peer=peer
        entry=self.entry
        self.record('observer','registration_location',dict(expected=str(entry),exists=entry.is_file(),
            fallback_home_entry_exists=(self.home/'.config/autostart/miopunch.desktop').is_file(),state=obj.call('state')['autostart']))
        assert entry.is_file(),'registration missing from actual XDG_CONFIG_HOME'
        assert obj.call('state')['autostart'] is True
        (self.output/'registered.desktop').write_bytes(entry.read_bytes())
        if self.case=='hidden':
            entry.write_text(entry.read_text()+'\nHidden=true\n')
            (self.output/'externally-disabled.desktop').write_bytes(entry.read_bytes())
            state=obj.call('state')
            self.record('observer','external_Hidden_state',dict(Hidden=True,actual_state=state['autostart']))
            assert state['autostart'] is False,'OS Hidden=true still reported enabled'
        obj.call('quit');obj.close()
        assert not self.app_processes()

    def configure_session(self):
        session_dir=Path('/usr/share/xsessions');session_dir.mkdir(exist_ok=True)
        wrapper=Path('/usr/local/bin/miopunch-validation-session')
        marker=self.output/'session-started.json'
        # This runs only as the session command selected by the real LightDM
        # PAM login. It records that identity before starting LXSession.
        observer=Path('/usr/local/bin/miopunch-session-observer.py')
        observer.write_text('import os,json,time;from pathlib import Path;Path('+repr(str(marker))+').write_text(json.dumps(dict(uid=os.getuid(),euid=os.geteuid(),pid=os.getpid(),parent=os.getppid(),DISPLAY=os.environ.get("DISPLAY"),XDG_SEAT=os.environ.get("XDG_SEAT"),clock=time.monotonic())))\n')
        system_xdg=self.private/'system-xdg';system_xdg.mkdir()
        wrapper.write_text('#!/bin/sh\nexport XDG_RUNTIME_DIR='+shlex.quote(str(self.runtime))+'\nexport XDG_CONFIG_HOME='+shlex.quote(str(self.config_home))+'\nexport XDG_CONFIG_DIRS='+shlex.quote(str(system_xdg))+'\nexport GDK_BACKEND=x11\n/usr/bin/python3 '+str(observer)+'\nexec /usr/bin/dbus-run-session -- /usr/bin/lxsession -s MIOPUNCH_VERIFY -e LXDE\n')
        wrapper.chmod(0o755)
        (session_dir/'miopunch-validation.desktop').write_text('[Desktop Entry]\nName=miopunch isolated login validation\nType=Application\nExec='+str(wrapper)+'\nDesktopNames=LXDE;\n')
        config=system_xdg/'lxsession/MIOPUNCH_VERIFY';config.mkdir(parents=True)
        (config/'desktop.conf').write_text('[Session]\nwindow_manager=openbox\ndisable_autostart=no\npolkit/command=\nclipboard/command=\n')
        conf=self.private/'lightdm.conf'
        conf.write_text('[LightDM]\nstart-default-seat=true\nminimum-vt=0\n[Seat:*]\ntype=xremote\nxserver-display-number=90\nautologin-user=ubuntu\nautologin-user-timeout=0\nuser-session=miopunch-validation\ngreeter-session=none\nallow-guest=false\n')
        return conf

    def udp_ports(self):
        result=set()
        for table in ['udp','udp6']:
            for line in (Path('/proc/net')/table).read_text().splitlines()[1:]:
                result.add((table,int(line.split()[1].split(':')[1],16)))
        return result

    def scenario(self):
        assert [name for _,name in socket.if_nameindex()]==['lo']
        Path('/run/dbus').mkdir(exist_ok=True)
        self.service('system-dbus',['dbus-daemon','--system','--nofork','--nopidfile'])
        xvfb=self.service('xvfb',['Xvfb',':90','-screen','0','1024x768x24','-nolisten','tcp','-ac'])
        until(lambda:Path('/tmp/.X11-unix/X90').exists(),'private Xvfb server absent')
        self.display=Display()
        bus=self.service('preparation-dbus',['dbus-daemon','--session','--nofork','--print-address=1'],user=1000,group=1000)
        until(lambda:(self.output/'preparation-dbus.stdout').read_text().startswith('unix:'),'private preparation session bus absent')
        address=(self.output/'preparation-dbus.stdout').read_text().splitlines()[0]
        self.initial(address)
        conf=self.configure_session()
        before=self.udp_ports()
        logs=self.output/'lightdm';logs.mkdir();os.chown(logs,1000,1000)
        dm=self.service('lightdm',['lightdm','--debug','--config',str(conf),'--log-dir',str(logs),
            '--run-dir',str(self.private/'dm-run'),'--cache-dir',str(self.private/'dm-cache'),'--pid-file',str(self.private/'dm.pid')])
        marker=self.output/'session-started.json'
        until(lambda:marker.exists(),'real PAM desktop session did not start',seconds=30)
        observed=json.loads(marker.read_text());assert observed['uid']==observed['euid']==1000
        self.record('observer','real_session_started',observed)
        if self.case=='hidden':
            start=time.monotonic()
            while time.monotonic()-start<5:
                assert not self.app_processes(),'OS Hidden=true unexpectedly started application'
                time.sleep(.1)
            self.record('observer','OS_Hidden_real_login_negative',dict(no_application=True,actual_session=observed,observed_seconds=5))
            dm.close()
            return
        until(lambda:bool(self.app_processes()),'registered login entry did not launch the actual application',seconds=20)
        applications=self.app_processes();assert len(applications)==1
        assert '--background' in applications[0]['args']
        assert self.display.windows()==[],'silent login created a window'
        after=self.udp_ports();ports=after-before
        until(lambda:bool(self.udp_ports()-before),'autostarted SDK socket absent',seconds=20)
        ports=self.udp_ports()-before
        address=dict(id=self.identity,addrs=[dict(Ip=(('[::1]:%d' if table=='udp6' else '127.0.0.1:%d')%port)) for table,port in sorted(ports)])
        assert self.peer.call('ping',address=address)['reply']=='PONG'
        self.record('observer','login_started_real_SDK',dict(process=applications[0],address=address,same_identity=True,authenticated_ping=True,no_window=True))
        owner=applications[0]
        code='import json,sys;from pathlib import Path;p=Path("/proc")/sys.argv[1];assert p.joinpath("stat").read_text().rsplit(")",1)[1].split()[19]==sys.argv[2];values=dict(v.decode().split("=",1) for v in p.joinpath("environ").read_bytes().split(b"\\0") if b"=" in v);names=["HOME","USER","LOGNAME","DISPLAY","XDG_RUNTIME_DIR","XDG_CONFIG_HOME","XDG_CONFIG_DIRS","DBUS_SESSION_BUS_ADDRESS","GDK_BACKEND","XAUTHORITY"];print(json.dumps({k:values[k] for k in names if k in values}))'
        observed_env=subprocess.run(['/usr/bin/python3','-B','-c',code,str(owner['pid']),owner['start_ticks']],capture_output=True,text=True,user=1000,group=1000)
        assert observed_env.returncode==0,observed_env.stderr
        login_env=self.gui_environment('unused');login_env.update(json.loads(observed_env.stdout))
        self.record('observer','login_owner_GUI_environment',json.loads(observed_env.stdout))
        with (self.output/'secondary.stdout').open('w') as stdout,(self.output/'secondary.stderr').open('w') as stderr:
            secondary=subprocess.Popen([str(self.desktop),'--data-dir',str(self.node_directory)],env=login_env,user=1000,group=1000,stdout=stdout,stderr=stderr)
            qualified=register_process(secondary,self.output/'secondary-owner.jsonl')
            assert secondary.wait(timeout=15)==0
            qualified.close()
        until(lambda:bool(self.display.windows()),'login owner window did not reopen')
        assert self.app_processes()==applications,'reopening changed the login owner'
        window=self.display.windows()[0]
        xpid=subprocess.check_output(['xprop','-display',':90','-id',hex(window),'_NET_WM_PID'],text=True)
        assert xpid.strip().endswith('= '+str(owner['pid'])),xpid
        until(lambda:self.display.checkbox_rendered_on(window),'actual WebView checked control not rendered',seconds=15)
        self.record('observer','actual_UI_ready',dict(owned_window=window,owned_PID=owner['pid'],checked_control_painted=True))
        # Only the fresh private Xvfb display is addressed. The checkbox and
        # quit button belong to the bundled, unchanged technical surface.
        self.display.click(window,130,60)
        entry=self.entry
        until(lambda:not entry.exists(),'real UI autostart-off did not remove its entry')
        self.display.click(window,500,60)
        until(lambda:not self.app_processes(),'real UI quit did not end the login owner',seconds=20)
        assert owner['parent']==os.getpid(),'login owner is not the original adopted child'
        waited,status=os.waitpid(owner['pid'],0)
        assert waited==owner['pid'] and os.WIFEXITED(status) and os.WEXITSTATUS(status)==0
        self.record('observer','login_owner_window_and_quit',dict(same_process=True,autostart_disabled=True,actual_UI_quit=True,
            process_absent=True,actual_exit_code=os.WEXITSTATUS(status),original_adopted_child_reaped=True))
        dm.close()
        marker.unlink()
        logs2=self.output/'lightdm-off';logs2.mkdir();os.chown(logs2,1000,1000)
        off=self.service('lightdm-off',['lightdm','--debug','--config',str(conf),'--log-dir',str(logs2),
            '--run-dir',str(self.private/'dm-off-run'),'--cache-dir',str(self.private/'dm-off-cache'),'--pid-file',str(self.private/'dm-off.pid')])
        until(lambda:marker.exists(),'second real PAM desktop session did not start',seconds=30)
        off_session=json.loads(marker.read_text());assert off_session['uid']==off_session['euid']==1000 and off_session['parent']!=observed['parent']
        start=time.monotonic()
        while time.monotonic()-start<5:
            assert not self.app_processes(),'disabled login unexpectedly started application'
            time.sleep(.1)
        self.record('observer','disabled_real_login_negative',dict(session=off_session,no_application=True,no_autostart_entry=not entry.exists(),observed_seconds=5))
        off.close()

    def run(self):
        report=dict(status='failed',clock_start=time.monotonic(),case=self.case)
        try:
            self.scenario();report['status']='pass'
        except BaseException as failure:
            report.update(failure_type=type(failure).__name__,failure=str(failure),traceback=traceback.format_exc())
        finally:
            cleanup=[];errors=[]
            for owner in list(reversed(self.desktops))+list(reversed(self.nodes)):
                try:cleanup.append(owner.close())
                except BaseException as failure:errors.append(dict(type=type(failure).__name__,message=str(failure)))
            remaining=self.app_processes()
            if remaining:
                # Preserve the first failure and keep the owned desktop alive
                # for normal UI settlement; do not stop its display manager
                # underneath an SDK owner or let container exit kill it.
                (self.output/'unsettled.json').write_text(json.dumps(dict(original_failure=report,applications=remaining),indent=2)+'\n')
                while self.app_processes():time.sleep(.2)
            for owner in reversed(self.services):
                try:cleanup.append(owner.close())
                except BaseException as failure:errors.append(dict(type=type(failure).__name__,message=str(failure)))
            for desktop in self.desktops:
                source=desktop.verification/'events.jsonl'
                if source.exists():(self.output/(desktop.label+'-native-events.jsonl')).write_bytes(source.read_bytes())
            report.update(cleanup=cleanup,cleanup_errors=errors,remaining_app_processes=self.app_processes(),clock_end=time.monotonic(),
                controller_drift=[p for p,h in self.controller_sources.items() if sha(p)!=h],
                memory_peak_bytes=int(Path('/sys/fs/cgroup/memory.peak').read_text()))
            if errors or report['remaining_app_processes'] or report['controller_drift']:report['status']='failed'
            (self.output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report));return 0 if report['status']=='pass' else 1


def outer(args):
    args.output.mkdir(exist_ok=False)
    private=Path(tempfile.mkdtemp(prefix='m121-linux-login-'));(private/'home').mkdir(mode=0o700)
    (private/'.owner').write_text(str(args.output.resolve()))
    desktop=json.loads(args.desktop_receipt.read_text())
    assert desktop['exit']==0 and sha(desktop['pin'])==desktop['sha256']
    from pin_artifact import pin_artifact
    deployed=pin_artifact(Path(desktop['pin']),private/case_paths(args.case)[0]/'miopunch-desktop',max_bytes=38*1024**3)
    image=args.image
    command=['rtk','proxy','docker','run','--rm','--cidfile',str(args.output/'container.cid'),'--network','none','--memory','2g','--pids-limit','512',
        '--user','0:0','--entrypoint','/usr/bin/python3','-v',str(ROOT)+':'+str(ROOT)+':ro','-v',str(private)+':'+str(private),
        '-v',str(private/'home')+':/home/ubuntu','-v',str(args.output)+':'+str(args.output),
        '-v',str(args.desktop_receipt)+':'+str(args.desktop_receipt)+':ro','-v',str(args.sdk_receipt)+':'+str(args.sdk_receipt)+':ro',
        '-w',str(ROOT),image,'-B',str(Path(__file__).resolve()),'--inside','--private',str(private),'--output',str(args.output/'case'),
        '--desktop-receipt',str(args.desktop_receipt),'--sdk-receipt',str(args.sdk_receipt),'--case',args.case]
    (args.output/'launch.json').write_text(json.dumps(dict(command=command,private=str(private),source_sha256=sha(__file__),
        deployment_alias=str(deployed),deployment_sha256=sha(deployed),origin_receipt=str(args.desktop_receipt),SDK_rebuilt=False),indent=2)+'\n')
    with (args.output/'controller.stdout').open('w') as stdout,(args.output/'controller.stderr').open('w') as stderr:
        result=subprocess.run(command,stdout=stdout,stderr=stderr)
    (args.output/'outer-result.json').write_text(json.dumps(dict(exit=result.returncode,private=str(private),container_cid=(args.output/'container.cid').read_text().strip()),indent=2)+'\n')
    print(dict(exit=result.returncode,private=str(private)));return result.returncode


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inside',action='store_true');parser.add_argument('--private',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--desktop-receipt',type=Path,required=True);parser.add_argument('--sdk-receipt',type=Path,required=True)
    parser.add_argument('--case',choices=['plain','spaces','xdg','hidden','quoting'],default='plain');parser.add_argument('--image')
    args=parser.parse_args();raise SystemExit(Runner(args).run() if args.inside else outer(args))
