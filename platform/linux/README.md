# Linux headless host

`miopunch-client serve DATA_DIR CONTROL_SOCKET` opens an existing native node
without a window or stdin dependency. It uses the same `ClientHost` and public
commands as the installed hosts. A closed control connection leaves the node
running. `stop`, node replacement, SIGTERM and SIGINT close the SDK before the
service exits.

Create or join the node with the existing `miopunch-client` JSON-lines entry
first, including the explicit recovery-card save/acknowledgement for a new
network. Stop that CLI before starting the service. CLI, headless and installed
hosts share a canonical directory lease, including symlink aliases. This is
cooperative local ownership, not protection against copying an identity to
another directory or device.

The [user service template](miopunch-headless.service) expects the verified
executable at `~/.local/bin/miopunch-client` and node state at
`~/.local/share/miopunch/node`. Install the template into
`~/.config/systemd/user/miopunch-headless.service`, then use:

```sh
systemctl --user daemon-reload
systemctl --user enable --now miopunch-headless.service
systemctl --user status miopunch-headless.service
```

The unit follows the user manager's lifecycle. For a dedicated always-on
deployment, configure that manager's startup according to the machine's service
policy. The application does not change system login or linger settings.

The control socket is `%t/miopunch/control.sock`, owned by the same user, mode
0600, inside a 0700 runtime directory. Each connection accepts one newline-ended
JSON request, at most 65,536 bytes. Malformed/oversized requests fail; ordinary
commands have 16 execution slots, while stop has independent execution capacity.
There are at most 32 accepted control connections and a five-second framing
timeout. The local channel is a trusted application-management interface.

For example, query the actual node:

```python
import json, os, socket

with socket.socket(socket.AF_UNIX) as control:
    control.connect(f"/run/user/{os.getuid()}/miopunch/control.sock")
    control.sendall(b'{"id":1,"op":"status"}\n')
    print(json.loads(control.makefile("rb").readline()))
```

Use `{"id":2,"op":"stop"}` or `systemctl --user stop miopunch-headless`
for a normal shutdown. A successful stop reply follows actual SDK close. The
host removes only its own socket inode. It refuses an existing socket path;
systemd owns runtime-directory cleanup. A manually launched crashed instance's
stale socket requires checking that its owner has exited before removing it.
Never delete the sibling `.host-lock` file to bypass an active owner.

The current bounded qualification covers real SDK admission, 14 duplicate-owner
rejections, controller EOF, an independently read 65,539-byte saved file, pending
HTTP cancellation, same-identity reopen, graceful SIGTERM and a real transient
user systemd unit. It does not establish an actual login trigger. See
[the source/pin and evidence record](../../docs/evidence/platform-host-2026-10-29/headless-qualification.json).
