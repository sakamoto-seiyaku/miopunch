#!/usr/bin/env python3
"""MIOPUNCH-63 full scheduler kernel fixture. Existing 57/58/62 artifacts stay frozen."""
import argparse, os, pathlib, subprocess, tempfile
from legacy_nat_lab import adapted_fixture, replace_once

def fixture(repo,topology,dialer,history):
    fallback=topology=="fallback"
    if fallback: topology="eim"
    dual="-" in topology
    if dual:
        source=(repo/"scripts/nat4_pair_lab.sh").read_text()
        source=replace_once(source,"strategy=${MIOPUNCH_NAT_STRATEGY:-auto}","strategy=legacy")
        source=replace_once(source,"topology=${MIOPUNCH_NAT_TOPOLOGY:-up1-up1}",f"topology={topology}")
        source=replace_once(source,"dialer=${MIOPUNCH_NAT_DIALER:-a}",f"dialer={dialer}")
        source=replace_once(source,"baseline|auto)","legacy)")
        source=source.replace("if [[ $strategy == auto ]]; then","if [[ $strategy == legacy ]]; then")
        source=replace_once(source,'strategy_env=()\nif [[ $strategy == baseline ]]; then strategy_env=(MIOPUNCH_NAT_STRATEGY=baseline); fi',f'strategy_env=(MIOPUNCH_NAT_STRATEGY=legacy MIOPUNCH_LEGACY_PREVIOUS_FAILURES={history})')
    else:
        custom={"spread5":([42031,42034,42032,42036],42035),"lowedge":([4,3,2,1],2),"highedge":([65532,65533,65534,65535],65534)}
        if topology in custom:
            ports,peer=custom[topology]
            source=adapted_fixture(repo,"nonmonotone",dialer).replace("nonmonotone",topology)
            source=replace_once(source,"observed_ports=(42031 42033 42032 42031)","observed_ports=("+" ".join(map(str,ports))+")")
            source=replace_once(source,"peer_port=$((base + 4 * step))",f"peer_port={peer}")
        else:
            source=adapted_fixture(repo,topology,dialer)
    if fallback: source=source.replace("if [[ $strategy == legacy && ${MIOPUNCH_LEGACY_FALLBACK:-0} != 1 ]]; then","if false; then")
    if dual:
        b_anchor='send_udp "$nat_a" 100.64.27.2 "$a_peer" "100.64.27.6:$b_peer" B-RIGHT-PORT'
        a_anchor='send_udp "$nat_b" 100.64.27.6 "$b_peer" "100.64.27.2:$a_peer" A-RIGHT-PORT'
        b_source='"$a_peer"';a_target='"100.64.27.2:$a_peer"';a_source='"$b_peer"'
    else:
        b_anchor='send_udp "$nat_a" 100.64.27.2 "$peer_port" 100.64.27.6:43027 B-RIGHT-PORT'
        a_anchor='send_udp "$nat_b" 100.64.27.6 43027 "100.64.27.2:$peer_port" A-RIGHT-PORT'
        b_source='"$peer_port"';a_target='"100.64.27.2:$peer_port"';a_source='43027'
    b_target='"100.64.27.6:$b_peer"' if dual else '100.64.27.6:43027'
    source=replace_once(source,b_anchor,f'ip netns exec "$nat_b" iptables -Z FORWARD\nsend_udp "$wan" 100.64.27.1 {b_source} {b_target} B-WRONG-IP\nsleep 0.1\nip netns exec "$nat_b" iptables -nvL FORWARD > "$evidence/filter-b-wrong-ip.txt"\nassert_dropped "$evidence/filter-b-wrong-ip.txt"\n'+b_anchor)
    chain='FORWARD' if dual else 'INPUT'
    source=replace_once(source,a_anchor,f'ip netns exec "$nat_a" iptables -Z {chain}\nsend_udp "$wan" 100.64.27.1 {a_source} {a_target} A-WRONG-IP\nsleep 0.1\nip netns exec "$nat_a" iptables -nvL {chain} > "$evidence/filter-a-wrong-ip.txt"\nassert_dropped "$evidence/filter-a-wrong-ip.txt"\n'+a_anchor)
    source=source[:source.rindex('case "$case_name" in\n')]
    # Emulate the old Internet hop loss before the receiving NAT's conntrack.
    anchor='  ip netns exec "$nat" iptables -P FORWARD DROP'
    source=replace_once(source,anchor,anchor+"\n  ip netns exec \"$nat\" iptables -t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP")
    source=replace_once(source,': > "$evidence/tcpdump.log"',': > "$evidence/tcpdump.log"\nfor side in a b; do\n  node_var="node_$side"\n  ip netns exec "${!node_var}" tcpdump --immediate-mode -U -n -i eth0 -w "$evidence/lan-$side.pcap" udp > "$evidence/tcpdump-lan-$side.log" 2>&1 &\n  jobs+=("$!")\ndone')
    source=source.replace('    ip netns exec "$nat_a" iptables -t nat -nvL POSTROUTING', '    ip netns exec "$nat_a" iptables -t raw -nvL PREROUTING > "$evidence/nat-a-raw.txt" || true\n    ip netns exec "$nat_a" iptables -t nat -nvL POSTROUTING',1)
    source=source.replace('    ip netns exec "$nat_b" iptables -t nat -nvL POSTROUTING', '    ip netns exec "$nat_b" iptables -t raw -nvL PREROUTING > "$evidence/nat-b-raw.txt" || true\n    ip netns exec "$nat_b" iptables -t nat -nvL POSTROUTING',1)
    source=source.replace('MIOPUNCH_NAT_STRATEGY="$strategy"',f'MIOPUNCH_LEGACY_PREVIOUS_FAILURES={history} MIOPUNCH_LEGACY_TTL_UNAVAILABLE="${{MIOPUNCH_LEGACY_TTL_UNAVAILABLE:-0}}" MIOPUNCH_NAT_STRATEGY="$strategy"')
    source+='\nif [[ $case_name == positive ]]; then\n  [[ $client_rc == 0 && $server_rc == 0 ]]\n  for side in client server; do\n    grep -q \'^LEGACY_SELECTED \' "$evidence/$side.log"\n    if grep -q DATA_OK "$evidence/$side.log"; then\n      grep -q CompleteExecution "$evidence/$side.log"\n    else\n      grep -q \'^PATH_UNAVAILABLE reason=legacy NAT phase budget elapsed\' "$evidence/$side.log"\n      grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/$side.log"\n    fi\n  done\nelse\n  if [[ $case_name == nonmember ]]; then\n    [[ $client_rc == 0 && $server_rc == 0 ]]\n    grep -Eq \'^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=[0-9]+ probes=0$\' "$evidence/client.log"\n    grep -Eq \'^ADMISSION_REJECTED peer=[0-9a-f]{64} stable_id=[0-9]+ probes=0$\' "$evidence/server.log"\n  elif [[ $case_name == revoked || $case_name == no_secret || $case_name == wrong_secret ]]; then\n    [[ $client_rc == 1 && $server_rc == 1 ]]\n    grep -Fxq \'Error: Connection was rejected locally\' "$evidence/client.log"\n    grep -Fxq \'Error: Custom { kind: Other, error: Connection was rejected locally }\' "$evidence/server.log"\n  else\n    [[ $client_rc == 0 && $server_rc == 0 ]]\n    grep -q PATH_UNAVAILABLE "$evidence/client.log"\n    grep -q PATH_UNAVAILABLE "$evidence/server.log"\n    grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/client.log"\n    grep -q CONTROL_ALIVE_AFTER_UDP_FAILURE "$evidence/server.log"\n  fi\n  ! grep -q DATA_OK "$evidence/client.log"\n  ! grep -q DATA_OK "$evidence/server.log"\nfi\necho "PASS scheduler case=$case_name topology=$topology dialer=$dialer history=$MIOPUNCH_LEGACY_PREVIOUS_FAILURES"\n'
    if fallback: source=source.replace('"$case_name" "$strategy" "$topology" "$dialer"','"$case_name" "$strategy" "fallback" "$dialer"')
    source=source.replace('history=$MIOPUNCH_LEGACY_PREVIOUS_FAILURES',f'history={history}')
    return source

def main():
    p=argparse.ArgumentParser();p.add_argument("case",choices=["positive","blocked","nonmember","revoked","no_secret","wrong_secret"]);p.add_argument("topology");p.add_argument("dialer",choices=["a","b"]);p.add_argument("history",type=int);p.add_argument("probe",type=pathlib.Path);p.add_argument("qad",type=pathlib.Path);p.add_argument("evidence",type=pathlib.Path)
    a=p.parse_args()
    if os.geteuid()!=0:p.error("run in disposable privileged NAT-lab container")
    source=fixture(pathlib.Path(__file__).resolve().parents[1],a.topology,a.dialer,a.history)
    a.evidence.mkdir(parents=True,exist_ok=True);(a.evidence/"fixture.sh").write_text(source)
    (a.evidence/"history.txt").write_text(str(a.history)+"\n")
    with tempfile.NamedTemporaryFile(mode="w",suffix=".sh",prefix="miopunch63-") as f:
        f.write(source);f.flush();r=subprocess.run(["bash",f.name,a.case,str(a.probe),str(a.qad),str(a.evidence)])
    raise SystemExit(r.returncode)
if __name__=="__main__":main()
