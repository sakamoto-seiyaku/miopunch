#!/usr/bin/env python3
"""Meaningful checker faults on disposable copies, never canonical evidence."""
import argparse,json,pathlib,shutil,struct,sys,tempfile
from legacy_nat_scheduler_verify import verify
from legacy_nat_scheduler_matrix import cells,name

def edit(path,old,new):
    text=path.read_text()
    if old not in text:raise RuntimeError("negative mutation did not match: "+old)
    path.write_text(text.replace(old,new,1))
def corrupt_raw_ttl(path):
    data=bytearray(path.read_bytes());link=struct.unpack("<I",data[20:24])[0];offset=20 if link==276 else 14;pos=24
    while pos<len(data):
        size=struct.unpack("<I",data[pos+8:pos+12])[0];start=pos+16;ip=start+offset;ihl=(data[ip]&15)*4
        if data[ip+9]==17 and data[ip+ihl+8:ip+ihl+19]==b"\0MIOPUNCH63":data[ip+8]=1;path.write_bytes(data);return
        pos=start+size
    raise RuntimeError("no raw datagram to corrupt")

def main():
    p=argparse.ArgumentParser();p.add_argument("evidence",type=pathlib.Path);a=p.parse_args();repo=pathlib.Path(__file__).resolve().parents[1]
    cell=cells()[0];baseline=a.evidence/"canonical"/name(cell);oracle=json.loads((repo/"tests/fixtures/legacy_nat_behaviors.json").read_text())
    verify(baseline,cell,oracle)
    mutations={
        "unexpected_exit":lambda d:edit(d/"run.txt","client_rc=0","client_rc=2"),
        "unrelated_runtime_error":lambda d:(d/"server.log").write_text((d/"server.log").read_text()+"Error: UDP send failed: permission denied\n"),
        "wrong_selected_index":lambda d:edit(d/"server.log","LEGACY_SELECTED mode=0 index=0","LEGACY_SELECTED mode=0 index=1"),
        "missing_sample":lambda d:edit(d/"server.log","LEGACY_SAMPLE order=3","REMOVED_SAMPLE order=3"),
        "missing_capture_drain":lambda d:edit(d/"tcpdump.log","0 packets dropped by kernel","1 packets dropped by kernel"),
        "header_only_pcap":lambda d:(d/"wan.pcap").write_bytes((d/"wan.pcap").read_bytes()[:24]),
        "truncated_pcap":lambda d:(d/"wan.pcap").write_bytes((d/"wan.pcap").read_bytes()[:-1]),
        "wrong_logged_ttl":lambda d:edit(d/"client.log","ttl=7 response=false","ttl=4 response=false"),
        "wrong_packet_ttl":lambda d:corrupt_raw_ttl(d/"lan-a.pcap"),
        "missing_raw_ttl_rule":lambda d:edit(d/"fixture.sh","-t raw -A PREROUTING -i wan0 -p udp -m ttl --ttl-lt 8 -j DROP","REMOVED_RAW_RULE"),
        "missing_wrong_ip_proof":lambda d:(d/"filter-a-wrong-ip.txt").unlink(),
        "changed_connection_id":lambda d:edit(d/"server.log","LEGACY_WINNER round=","LEGACY_WINNER stable_id=99 round="),
        "missing_phase_cleanup":lambda d:edit(d/"server.log","LEGACY_PHASE_STOP","REMOVED_PHASE_STOP"),
        "wrong_business_peer":lambda d:edit(d/"server.log","DATA_OK stable_id=","DATA_OK peer=wrong stable_id="),
    }
    rows=[]
    with tempfile.TemporaryDirectory(prefix="mio63-checker-negatives-") as tmp:
        for label,mutation in mutations.items():
            directory=pathlib.Path(tmp)/label;shutil.copytree(baseline,directory);mutation(directory)
            try:verify(directory,cell,oracle)
            except (ValueError,FileNotFoundError) as error:rows.append(dict(case=label,result="REJECT",reason=str(error)))
            else:raise RuntimeError("checker accepted negative: "+label)
    print(json.dumps(dict(result="PASS",rejected=len(rows),rows=rows),indent=2))
if __name__=="__main__":main()
