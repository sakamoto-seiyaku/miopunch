"""Unprivileged PTY/tmux adapter. One helper and socket per admitted peer.

The Rust provider drops UID/GID before executing this process. It has no access
to the node's private state or the Docker socket. Output retention is bounded.
"""
import base64
import collections
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import threading
import time

directory = Path('/workspace') / sys.argv[1]
directory.mkdir(mode=0o700, parents=True, exist_ok=True)
os.chdir(directory)
os.environ.update(HOME=str(directory), TERM='xterm-256color', SHELL='/bin/bash')
socket_path = str(directory / 'tmux.sock')
prefix = ['tmux', '-S', socket_path]
attached = {}
lock = threading.Lock()
limit = 256 * 1024


def tmux(*args):
    return subprocess.run(prefix + list(args), capture_output=True, text=True)


def sessions():
    result = tmux('list-sessions', '-F', '#{session_name}')
    return result.stdout.splitlines() if result.returncode == 0 else []


def attach(name, cols=100, rows=28):
    if name in attached:
        return
    if name not in sessions():
        subprocess.run(prefix + ['new-session', '-d', '-s', name, '-x', str(cols), '-y', str(rows), '/bin/bash'], check=True, capture_output=True)
        tmux('set-option', '-g', 'history-limit', '3000')
        tmux('set-option', '-g', 'status', 'off')
        tmux('set-option', '-g', 'mouse', 'on')
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp('tmux', prefix + ['attach-session', '-t', name])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
    entry = {'fd': fd, 'pid': pid, 'chunks': collections.deque(), 'start': 0, 'end': 0, 'closed': False}
    attached[name] = entry

    def drain():
        try:
            while not entry['closed']:
                data = os.read(fd, 8192)
                if not data:
                    break
                with lock:
                    entry['chunks'].append(data)
                    entry['end'] += len(data)
                    while entry['end'] - entry['start'] > limit:
                        entry['start'] += len(entry['chunks'].popleft())
        except OSError:
            pass

    threading.Thread(target=drain, daemon=True).start()


def valid_name(request):
    name = request.get('name', '')
    if not name.startswith('term-') or len(name) > 40 or not all(x.isalnum() or x == '-' for x in name):
        raise ValueError('Invalid terminal session')
    return name


def handle(request):
    action = request['action']
    if action == 'list':
        return {'sessions': sessions()}
    name = valid_name(request)
    if action == 'attach':
        if name not in sessions() and len(sessions()) >= 6:
            raise ValueError('At most six terminal workspaces')
        attach(name)
        with lock:
            history_end = attached[name]['end']
        return {'name': name, 'offset': 0, 'historyEnd': history_end}
    if action == 'end':
        tmux('kill-session', '-t', name)
    if action in ('detach', 'end'):
        entry = attached.pop(name, None)
        if entry:
            entry['closed'] = True
            # Stop this attached tmux CLIENT. Closing the PTY while the reader
            # thread is blocked does not reliably wake it; waiting for that
            # client first would hang. The tmux SERVER/workspace stays alive.
            try:
                os.kill(entry['pid'], signal.SIGTERM)
            except ProcessLookupError:
                pass
            os.close(entry['fd'])
            try:
                until = time.monotonic() + 1
                while os.waitpid(entry['pid'], os.WNOHANG)[0] == 0:
                    if time.monotonic() >= until:
                        os.kill(entry['pid'], signal.SIGKILL)
                        os.waitpid(entry['pid'], 0)
                        break
                    time.sleep(0.01)
            except ChildProcessError:
                pass
        return {'detached': True}
    if name not in attached:
        raise ValueError('Attach this terminal first')
    entry = attached[name]
    if action == 'write':
        data = base64.b64decode(request.get('data', ''), validate=True)
        if len(data) > 8192:
            raise ValueError('Terminal input too large')
        os.write(entry['fd'], data)
        return {'written': len(data)}
    if action == 'resize':
        cols = max(20, min(240, int(request['cols'])))
        rows = max(5, min(100, int(request['rows'])))
        fcntl.ioctl(entry['fd'], termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
        return {'resized': True}
    if action == 'poll':
        offset = int(request.get('offset', 0))
        with lock:
            start = max(offset, entry['start'])
            data = b''.join(entry['chunks'])[start - entry['start']:][:24000]
            return {'data': base64.b64encode(data).decode(), 'offset': start + len(data), 'truncated': offset < entry['start']}
    raise ValueError('Unknown terminal action')


try:
    for line in sys.stdin:
        try:
            result = handle(json.loads(line))
        except Exception as exc:
            result = {'error': str(exc)}
        print(json.dumps(result), flush=True)
finally:
    for entry in attached.values():
        try:
            os.close(entry['fd'])
        except OSError:
            pass
    # tmux servers/workspaces deliberately survive UI/helper disconnection.
