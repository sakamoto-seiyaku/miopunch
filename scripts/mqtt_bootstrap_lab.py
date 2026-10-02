#!/usr/bin/env python3
"""Own a disposable dual-NAT fixture and use the formal native node test entry."""
import pathlib
import subprocess
import sys
import tempfile


def main():
    worker, qad, output = map(pathlib.Path, sys.argv[1:4])
    case = sys.argv[4]
    repo = pathlib.Path(__file__).resolve().parents[1]
    source = (repo / 'scripts/udp_nat_lab.sh').read_text()
    source = source[:source.index('client_id=$(')]
    source = source.replace('/tmp/miopunch27.', '/tmp/miopunch42.').replace('prefix="m27$$"', 'prefix="m42$$"')
    source = source.replace('100.64.27.1:46761', '100.64.27.1:7842')
    source = source.replace('positive|no_prime|path_id_retry|blocked|wrong|stale|control_cut)', 'positive|duplicate|data_cut|no_prime|path_id_retry|blocked|wrong|stale|control_cut)')
    source = source.replace('  for ns in "${names[@]}"; do', '''  for nat in "$nat_a" "$nat_b"; do
    ip netns exec "$nat" iptables-save -c > "$evidence/$nat-rules.txt" 2>/dev/null || true
    ip netns exec "$nat" conntrack -L > "$evidence/$nat-conntrack.txt" 2>&1 || true
  done
  for ns in "${names[@]}"; do''', 1)
    source += r'''
cp "$runtime/cert.der" "$runtime/nodes/cert.der"
if [[ $case_name == duplicate || $case_name == data_cut ]]; then
  ip netns exec "$wan" env MIO42_CASE="$case_name" MIO42_ROLE=observe MIO42_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact mqtt_native_node_worker --nocapture > "$evidence/observer.log" 2>&1 &
  jobs+=("$!")
  for _ in $(seq 1 100); do
    [[ -f "$runtime/nodes/observer-ready" ]] && break
    sleep 0.01
  done
  [[ -f "$runtime/nodes/observer-ready" ]]
fi
ip netns exec "$node_b" env MIO42_CASE="$case_name" MIO42_ROLE=serve MIO42_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact mqtt_native_node_worker --nocapture > "$evidence/server.log" 2>&1 &
server_pid=$!
jobs+=("$server_pid")
for _ in $(seq 1 100); do
  [[ -f "$runtime/nodes/server-route" ]] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -f "$runtime/nodes/server-route" ]]
if [[ $case_name == control_cut || $case_name == data_cut ]]; then
  ip netns exec "$node_a" env MIO42_CASE="$case_name" MIO42_ROLE=connect MIO42_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact mqtt_native_node_worker --nocapture > "$evidence/client.log" 2>&1 &
  client_pid=$!
  jobs+=("$client_pid")
  if [[ $case_name == control_cut ]]; then
    for _ in $(seq 1 100); do
      grep -q MQTT_BOOTSTRAP_START "$evidence/client.log" && break
      kill -0 "$client_pid" 2>/dev/null || break
      sleep 0.01
    done
    grep -q MQTT_BOOTSTRAP_START "$evidence/client.log"
    kill "${jobs[0]}"
  else
    for _ in $(seq 1 500); do
      [[ -f "$runtime/nodes/client-cut-ready" && -f "$runtime/nodes/server-cut-ready" ]] && break
      kill -0 "$client_pid" 2>/dev/null || break
      sleep 0.01
    done
    [[ -f "$runtime/nodes/client-cut-ready" && -f "$runtime/nodes/server-cut-ready" ]]
    ip netns exec "$wan" iptables -I FORWARD 1 -p udp -s 100.64.27.2 -d 100.64.27.6 -j DROP
    ip netns exec "$wan" iptables -I FORWARD 1 -p udp -s 100.64.27.6 -d 100.64.27.2 -j DROP
    date -u +'%s.%N' > "$evidence/data-cut-clock.txt"
    touch "$runtime/nodes/data-cut"
  fi
  wait "$client_pid"
else
  ip netns exec "$node_a" env MIO42_CASE="$case_name" MIO42_ROLE=connect MIO42_DIRECTORY="$runtime/nodes" timeout 40 "$probe" --exact mqtt_native_node_worker --nocapture > "$evidence/client.log" 2>&1
fi
wait "$server_pid"
if [[ $case_name == control_cut || $case_name == blocked ]]; then
  grep -q CONTROL_UNAVAILABLE "$evidence/client.log"
  grep -q CONTROL_UNAVAILABLE "$evidence/server.log"
  ! grep -q DATA_OK "$evidence/client.log"
  ! grep -q DATA_OK "$evidence/server.log"
else
  grep -q DATA_OK "$evidence/client.log"
  grep -q DATA_OK "$evidence/server.log"
  if [[ $case_name == duplicate ]]; then
    grep -q MQTT_DUPLICATE "$evidence/observer.log"
    grep -q DUPLICATE_BOUNDED "$evidence/client.log"
    grep -q DUPLICATE_BOUNDED "$evidence/server.log"
  elif [[ $case_name == data_cut ]]; then
    grep -q DATA_FAILED "$evidence/client.log"
    grep -q DATA_FAILED "$evidence/server.log"
    ip netns exec "$wan" iptables-save -c > "$evidence/data-cut-rules.txt"
  fi
fi
echo "PASS formal MQTT dual NAT case=$case_name"
'''
    source = source.replace("printf 'listener 1883", '''env MIO42_ROLE=prepare MIO42_DIRECTORY="$runtime/nodes" "$probe" --exact mqtt_native_node_worker --nocapture > "$evidence/prepare.log" 2>&1
printf 'listener 1883''')
    output.mkdir(parents=True, exist_ok=True)
    (output / 'fixture.sh').write_text(source)
    with tempfile.NamedTemporaryFile(mode='w', suffix='.sh', prefix='mio42-') as script:
        script.write(source)
        script.flush()
        result = subprocess.run(['bash', script.name, case, str(worker), str(qad), str(output)])
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
