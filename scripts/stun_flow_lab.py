#!/usr/bin/env python3
"""One owned dual-kernel-NAT lifecycle: QAD, STUN backup, disable, restore."""
import pathlib
import subprocess
import sys
import tempfile
from tcp_punch_lab import fixture_source


def main():
    worker, qad, output = map(pathlib.Path, sys.argv[1:4])
    coalesced = sys.argv[4:] == ["coalesced"]
    repo = pathlib.Path(__file__).resolve().parents[1]
    source = fixture_source(repo)
    source = source.replace('case_name=$1', 'case_name=$1\nexport MIO67_FLOW=1', 1)
    start = source.index('ip netns exec "$node_b" env MIO43_CASE="$case_name" MIO43_ROLE=serve')
    source = source[:start] + r'''
ip netns exec "$wan" "$qad" serve "$runtime/cert.der" 0.0.0.0:7842 > "$evidence/qad.log" 2>&1 &
jobs+=("$!")
for _ in $(seq 1 100); do [[ -f "$runtime/cert.der" ]] && grep -q QAD_READY "$evidence/qad.log" && break; sleep 0.01; done
grep -q QAD_READY "$evidence/qad.log"
cp "$runtime/cert.der" "$runtime/nodes/cert.der"
for node in "$node_a" "$node_b"; do
  mkdir -p "/etc/netns/$node"
  printf '127.0.0.1 localhost\n100.64.27.1 qad.miopunch.test\n100.64.27.5 qad2.miopunch.test\n' > "/etc/netns/$node/hosts"
done
# Withhold only the initial background QAD report so direct LAN validation
# cannot preempt the sequence seam. The actual QAD service is healthy when
# the ordinary admitted path reaches its observation round.
ip netns exec "$wan" iptables -I INPUT 1 -p udp --dport 7842 -j DROP

run_phase() {
  local phase=$1
  local phase_dir="$evidence/$phase"
  mkdir "$phase_dir"
  rm -f "$runtime/nodes/server-route" "$runtime/nodes/client-route"
  date -u +%FT%TZ > "$phase_dir/started.txt"
  ip netns exec "$node_b" env MIO43_CASE=stun_udp MIO43_ROLE=serve MIO43_DIRECTORY="$runtime/nodes" MIO67_PHASE="$phase" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$phase_dir/server.log" 2>&1 &
  local server_pid=$!
  jobs+=("$server_pid")
  for _ in $(seq 1 100); do [[ -f "$runtime/nodes/server-route" ]] && break; kill -0 "$server_pid" || break; sleep 0.05; done
  [[ -f "$runtime/nodes/server-route" ]]
  ip netns exec "$node_a" env MIO43_CASE=stun_udp MIO43_ROLE=connect MIO43_DIRECTORY="$runtime/nodes" MIO67_PHASE="$phase" timeout 40 "$probe" --exact tcp_native_punch_worker --nocapture > "$phase_dir/client.log" 2>&1 &
  local client_pid=$!
  jobs+=("$client_pid")
  if [[ $phase == qad || $phase == coalesced ]]; then
    for _ in $(seq 1 500); do
      grep -q NAT_CONNECTION_ROUND "$phase_dir/client.log" && grep -q NAT_CONNECTION_ROUND "$phase_dir/server.log" && break
      kill -0 "$client_pid" && kill -0 "$server_pid" || break
      sleep 0.01
    done
    grep -q NAT_CONNECTION_ROUND "$phase_dir/client.log"
    grep -q NAT_CONNECTION_ROUND "$phase_dir/server.log"
    ip netns exec "$wan" iptables -D INPUT -p udp --dport 7842 -j DROP
  fi
  wait "$client_pid"
  wait "$server_pid"
  for node in "$node_a" "$node_b"; do
    ip netns exec "$node" ss -Huan > "$phase_dir/$node-udp-after.txt"
    ip netns exec "$node" ss -Htan state syn-sent > "$phase_dir/$node-tcp-pending-after.txt"
    [[ ! -s "$phase_dir/$node-udp-after.txt" ]]
    [[ ! -s "$phase_dir/$node-tcp-pending-after.txt" ]]
  done
  if [[ $phase == disabled ]]; then
    grep -q UNSUPPORTED_BOUNDED "$phase_dir/client.log"
    grep -q UNSUPPORTED_BOUNDED "$phase_dir/server.log"
    ! grep -q DATA_OK "$phase_dir/client.log"
    ! grep -q DATA_OK "$phase_dir/server.log"
    ! grep -q '^LEGACY_SAMPLE ' "$phase_dir/client.log"
    ! grep -q '^LEGACY_SAMPLE ' "$phase_dir/server.log"
    ! grep -q STUN_UDP_SEND "$phase_dir/client.log"
    ! grep -q STUN_UDP_SEND "$phase_dir/server.log"
  else
    grep -q DATA_OK "$phase_dir/client.log"
    grep -q DATA_OK "$phase_dir/server.log"
    grep -q STUN_FLOW_SYNC "$phase_dir/client.log"
    if [[ $phase == qad ]]; then
      grep -q '^LEGACY_SAMPLE .*observer=https://' "$phase_dir/client.log"
      grep -q '^LEGACY_SAMPLE .*observer=https://' "$phase_dir/server.log"
      ! grep -q STUN_UDP_SEND "$phase_dir/client.log"
      ! grep -q STUN_UDP_SEND "$phase_dir/server.log"
      ! grep -q STUN_UDP_BINDING "$evidence/udp-observer.log"
    else
      if [[ $phase != coalesced ]]; then
        grep -q QAD_SEQUENCE_UNAVAILABLE "$phase_dir/client.log"
        grep -q QAD_SEQUENCE_UNAVAILABLE "$phase_dir/server.log"
      fi
      [[ $(grep -c '^LEGACY_SAMPLE .*observer=stun://' "$phase_dir/client.log") == 4 ]]
      [[ $(grep -c '^LEGACY_SAMPLE .*observer=stun://' "$phase_dir/server.log") == 4 ]]
      grep -q '^LEGACY_FEATURE feature=Some' "$phase_dir/client.log"
      grep -q '^LEGACY_FEATURE feature=Some' "$phase_dir/server.log"
    fi
  fi
  date -u +%FT%TZ > "$phase_dir/finished.txt"
  echo "PASS STUN_FLOW phase=$phase"
}
run_phase qad
# Only QAD becomes unavailable; all STUN, signaling and peer routes stay allowed.
ip netns exec "$wan" iptables -I INPUT 1 -p udp --dport 7842 -j DROP
run_phase backup
env MIO43_CASE=stun_udp MIO43_ROLE=disable_stun MIO43_DIRECTORY="$runtime/nodes" "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/disable-config.log" 2>&1
run_phase disabled
env MIO43_CASE=stun_udp MIO43_ROLE=restore_stun MIO43_DIRECTORY="$runtime/nodes" "$probe" --exact tcp_native_punch_worker --nocapture > "$evidence/restore-config.log" 2>&1
run_phase restored
echo 'PASS QAD-primary STUN-backup priority-disable exact-restore same-identities'
'''
    if coalesced:
        source = source.replace("100.64.27.5 qad2.miopunch.test", "100.64.27.1 qad2.miopunch.test")
        source = source[:source.index("\nrun_phase qad\n")] + "\nrun_phase coalesced\necho 'PASS QAD DNS coalescence selects distinct STUN destinations'\n"
    output.mkdir(parents=True, exist_ok=True)
    (output / "fixture.sh").write_text(source)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".sh", prefix="mio67-") as script:
        script.write(source)
        script.flush()
        result = subprocess.run(["bash", script.name, "stun_udp", str(worker), str(qad), str(output)])
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
