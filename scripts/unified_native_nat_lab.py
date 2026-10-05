#!/usr/bin/env python3
"""Representative common-SDK native paths over two owned Linux kernel NATs.

Run inside the existing isolated privileged NAT container. This reuses the
already qualified topology, not the historical worker or product wire.
"""
import pathlib
import subprocess
import sys
import tempfile


def fixture_source(repo):
    source = (repo / "scripts/udp_nat_lab.sh").read_text()
    source = source[:source.index("# Endpoint-independent mapping")]
    source = source.replace("positive|no_prime|path_id_retry|blocked|wrong|stale|control_cut", "udp|tcp|fallback|blocked|asymmetric")
    source = source.replace("/tmp/miopunch27.", "/tmp/miopunch79.").replace('prefix="m27$$"', 'prefix="m79$$"')
    start = source.index("cleanup() {")
    end = source.index("trap cleanup EXIT")
    source = source[:start] + r'''
cleanup() {
  original_exit=$?
  set +e
  for nat in "$nat_a" "$nat_b"; do
    ip netns exec "$nat" iptables-save -c > "$evidence/$nat-rules.txt" 2>/dev/null
    ip netns exec "$nat" conntrack -L > "$evidence/$nat-conntrack.txt" 2>&1
  done
  for job in "${jobs[@]}"; do kill "$job" 2>/dev/null; done
  for job in "${jobs[@]}"; do wait "$job" 2>/dev/null; done
  for ns in "${names[@]}"; do
    ip netns del "$ns" 2>/dev/null
    rm -rf "/etc/netns/$ns"
  done
  rm -rf "$runtime"
  ip netns list > "$evidence/namespaces-after.txt"
  printf 'originalExit=%s\n' "$original_exit" > "$evidence/fixture-exit.txt"
  exit "$original_exit"
}
''' + source[end:]
    source += r'''
# Existing supported TCP NAT condition: silently discard unsolicited SYNs.
for nat in "$nat_a" "$nat_b"; do
  ip netns exec "$nat" iptables -A INPUT -i wan0 -p tcp --syn -j DROP
done
ip netns exec "$wan" sysctl -qw net.ipv4.ip_forward=1
if [[ $case_name == fallback || $case_name == blocked ]]; then
  for protocol in tcp udp; do
    if [[ $protocol == udp && $case_name == fallback ]]; then continue; fi
    ip netns exec "$wan" iptables -A FORWARD -p "$protocol" -s 100.64.27.2 -d 100.64.27.6 -j DROP
    ip netns exec "$wan" iptables -A FORWARD -p "$protocol" -s 100.64.27.6 -d 100.64.27.2 -j DROP
  done
fi
if [[ $case_name != tcp ]]; then
  # Qualified endpoint-independent mapping, address-dependent filtering fixture.
  for side in a b; do
    if [[ $side == a ]]; then nat="$nat_a"; private=10.27.1.2; public=100.64.27.2; port=42027;
    else nat="$nat_b"; private=10.27.2.2; public=100.64.27.6; port=43027; fi
    ip netns exec "$nat" iptables -t mangle -A FORWARD -i lan0 -o wan0 -p udp --sport "$port" -m recent --name mio79allow --set --rdest
    ip netns exec "$nat" iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$port" -j SNAT --to-source "$public:$port"
    ip netns exec "$nat" iptables -t nat -A PREROUTING -i wan0 -p udp --dport "$port" -j DNAT --to-destination "$private:$port"
    ip netns exec "$nat" iptables -I FORWARD 1 -i wan0 -o lan0 -p udp --dport "$port" -m recent --name mio79allow --rcheck --seconds 60 --rsource -j ACCEPT
  done
fi
"$probe" prepare "$runtime/nodes" "$case_name" > "$evidence/prepare.log" 2>&1
printf 'listener 1883 100.64.27.1\nallow_anonymous true\npersistence false\n' > "$runtime/mosquitto.conf"
ip netns exec "$wan" mosquitto -c "$runtime/mosquitto.conf" > "$evidence/broker.log" 2>&1 &
jobs+=("$!")
if [[ $case_name != udp ]]; then
  ip netns exec "$wan" python3 /repo/scripts/tcp_stun_observer.py > "$evidence/tcp-observer.log" 2>&1 &
  jobs+=("$!")
fi
if [[ $case_name != tcp ]]; then
  ip netns exec "$wan" python3 /repo/scripts/stun_udp_observer.py --bind 100.64.27.1:3478 --bind 100.64.27.1:3479 --bind 100.64.27.5:3478 --bind 100.64.27.5:3479 > "$evidence/udp-observer.log" 2>&1 &
  jobs+=("$!")
fi
ip netns exec "$wan" tcpdump --immediate-mode -U -n -s 256 -i any -w "$evidence/wan.pcap" 'tcp or udp' > "$evidence/tcpdump.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do
  ready=true
  grep -q 'Opening ipv4 listen socket' "$evidence/broker.log" || ready=false
  grep -q 'listening on' "$evidence/tcpdump.log" || ready=false
  if [[ $case_name != udp ]]; then grep -q STUN_TCP_READY "$evidence/tcp-observer.log" || ready=false; fi
  if [[ $case_name != tcp ]]; then [[ $(grep -c STUN_UDP_READY "$evidence/udp-observer.log") == 4 ]] || ready=false; fi
  if $ready; then break; fi
  sleep 0.05
done
$ready
ip netns exec "$node_b" timeout 60 "$probe" serve "$runtime/nodes/server" "$runtime/target.json" 43027 > "$evidence/server.log" 2>&1 &
server_pid=$!
jobs+=("$server_pid")
for _ in $(seq 1 200); do
  [[ -f "$runtime/target.json" ]] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.05
done
[[ -f "$runtime/target.json" ]]
set +e
ip netns exec "$node_a" timeout 60 "$probe" connect "$runtime/nodes/client" "$runtime/target.json" 42027 > "$evidence/client.log" 2>&1
client_exit=$?
wait "$server_pid"
server_exit=$?
set -e
printf 'clientExit=%s\nserverExit=%s\n' "$client_exit" "$server_exit" > "$evidence/actor-exits.txt"
if [[ $case_name == blocked ]]; then
  [[ $client_exit == 1 && $server_exit == 1 ]]
  grep -q '"ordinaryHandleReturned":false' "$evidence/client.log"
  grep -q '"ordinaryHandleReturned":false' "$evidence/server.log"
else
  [[ $client_exit == 0 && $server_exit == 0 ]]
  grep -q '"streamMatched":true' "$evidence/client.log"
  grep -q '"streamMatched":true' "$evidence/server.log"
fi
echo "PASS common SDK native NAT case=$case_name"
'''
    return source


def main():
    worker, output = map(pathlib.Path, sys.argv[1:3])
    case = sys.argv[3]
    if case not in {"udp", "tcp", "fallback", "blocked", "asymmetric"}:
        raise SystemExit("use udp, tcp, fallback, blocked or asymmetric")
    output.mkdir(parents=True, exist_ok=True)
    source = fixture_source(pathlib.Path(__file__).resolve().parents[1])
    (output / "fixture.sh").write_text(source)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".sh", prefix="mio79-") as script:
        script.write(source)
        script.flush()
        result = subprocess.run(["bash", script.name, case, str(worker), "unused", str(output)])
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
