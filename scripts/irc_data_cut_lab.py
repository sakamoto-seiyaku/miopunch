#!/usr/bin/env python3
"""Only runs inside the owned offline container; drops that fixture's UDP flows."""
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import socket


def main():
    worker = sys.argv[1]
    with tempfile.TemporaryDirectory(prefix='mio45-cut-') as temporary:
        directory = pathlib.Path(temporary)
        # Checked-in, non-secret fixture config; real IRCd, no simulated server.
        config = pathlib.Path('/repo/tests/fixtures/irc/ergo.yaml')
        with (pathlib.Path('/evidence') / 'ircd.log').open('w') as log:
            server = subprocess.Popen(['/ircd-bin/ergo', 'run', '--conf', str(config)],
                                      cwd=directory, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while True:
                    if server.poll() is not None:
                        raise RuntimeError('owned IRCd failed; read ircd.log')
                    try:
                        with socket.create_connection(('127.0.0.1', 6667), timeout=1):
                            break
                    except OSError:
                        if time.monotonic() > deadline:
                            raise
                        time.sleep(.1)
                environment = dict(os.environ, MIOPUNCH_IRC_SERVER='127.0.0.1:6667',
                                   MIO45_ISOLATED_NETFILTER='1')
                result = subprocess.run([worker, '--exact',
                                         'losing_the_udp_data_path_does_not_send_business_back_to_irc',
                                         '--ignored', '--nocapture'], env=environment, timeout=40)
                raise SystemExit(result.returncode)
            finally:
                server.terminate()
                server.wait(timeout=5)


if __name__ == '__main__':
    main()
