rtk proxy python - <<'PY_RAW'
import socket,os,struct,time,json
server=('74.125.250.129',19302)
rows=[]
for bind in ('0.0.0.0','192.168.4.5'):
 s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind((bind,0));s.settimeout(2)
 tid=os.urandom(12);request=struct.pack('!HHI',1,0,0x2112a442)+tid;t=time.monotonic();s.sendto(request,server)
 try:
  data,peer=s.recvfrom(1024);row=dict(bind=s.getsockname(),server=server,responsePeer=peer,responseBytes=len(data),responseType=struct.unpack('!H',data[:2])[0],transactionMatched=data[8:20]==tid,elapsedMs=round((time.monotonic()-t)*1000,3))
 except OSError as e: row=dict(bind=s.getsockname(),failure=str(e),elapsedMs=round((time.monotonic()-t)*1000,3))
 s.close();row['socketClosed']=True;rows.append(row)
open('/tmp/mio71-raw-stun.json','w').write(json.dumps(rows,indent=2)+'\n');print(json.dumps(rows,indent=2))
PY_RAW
