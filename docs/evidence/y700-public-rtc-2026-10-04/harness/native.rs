use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};
use serde_json::{json, Value};
use tokio::{io::{AsyncBufReadExt, BufReader}, sync::mpsc};
use bytes::BytesMut;
use webrtc::peer_connection::PeerConnection;
use webrtc::{data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit}, peer_connection::{PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceServer, RTCIceGatheringState, RTCIceConnectionState, RTCPeerConnectionState, RTCSessionDescription, SettingEngineBuilder}};

struct Handler { gathered: mpsc::Sender<()>, events: Arc<Mutex<Vec<Value>>>, started: Instant }
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
 async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) { let _ = dc.close().await; }
 async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
  self.events.lock().unwrap().push(json!({"kind":"gathering","state":state.to_string(),"ms":self.started.elapsed().as_millis()}));
  if state == RTCIceGatheringState::Complete { let _ = self.gathered.try_send(()); }
 }
 async fn on_ice_connection_state_change(&self, state: RTCIceConnectionState) { self.events.lock().unwrap().push(json!({"kind":"ice","state":state.to_string(),"ms":self.started.elapsed().as_millis()})); }
 async fn on_connection_state_change(&self, state: RTCPeerConnectionState) { self.events.lock().unwrap().push(json!({"kind":"connection","state":state.to_string(),"ms":self.started.elapsed().as_millis()})); }
}
fn emit(v: Value) { println!("{v}"); }
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
 let mut input = BufReader::new(tokio::io::stdin()).lines();
 let init: Value = serde_json::from_str(&input.next_line().await?.ok_or("input closed")?)?;
 let (gathered, mut receiver) = mpsc::channel(1);
 let events = Arc::new(Mutex::new(Vec::new()));
 let started = Instant::now();
 let observers = if init["stun"].as_str().unwrap_or("").is_empty() { vec![] } else { vec![RTCIceServer { urls: vec![init["stun"].as_str().unwrap().into()], username:String::new(), credential:String::new() }] };
 let pc = PeerConnectionBuilder::new()
  .with_configuration(RTCConfigurationBuilder::default().with_ice_servers(observers).build())
  .with_setting_engine(SettingEngineBuilder::default().with_sctp_max_receive_buffer_size(65536).build())
  .with_data_channel_send_buffer_limit(65536)
  .with_handler(Arc::new(Handler { gathered, events:events.clone(), started }))
  .with_udp_addrs(vec!["192.168.4.5:0"]).build().await?;
 let result = async {
  let dc = pc.create_data_channel("mio-quic-datagram/1", Some(RTCDataChannelInit { ordered:false, max_retransmits:Some(0), negotiated:Some(0), protocol:"miopunch/quic-datagram/1".into(), ..Default::default() })).await?;
  let offer: RTCSessionDescription = serde_json::from_value(init["offer"].clone())?;
  pc.set_remote_description(offer).await?;
  pc.set_local_description(pc.create_answer(None).await?).await?;
  tokio::time::timeout(Duration::from_secs(12), receiver.recv()).await?.ok_or("gathering stopped")?;
  emit(json!({"event":"answer","description":pc.local_description().await.ok_or("answer absent")?}));
  let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
  let mut matched = false;
  let mut sent = false;
  let mut opened = false;
  let mut notified = false;
  loop {
   tokio::select! {
    _ = tokio::time::sleep_until(deadline) => return Err("RTC data/control deadline".into()),
    line = input.next_line() => {
     let line = line?.ok_or("test controller closed")?;
     if line=="send" && opened && !sent { dc.try_send(BytesMut::from(vec![0xa7u8;1200].as_slice())).await?;sent=true; }
     else if line=="finish" && matched && sent { break; }
     else { return Err("unexpected local test control".into()); }
    }
    value = dc.poll() => match value {
     Some(DataChannelEvent::OnOpen) => { opened=true;emit(json!({"event":"open","ms":started.elapsed().as_millis()})); }
     Some(DataChannelEvent::OnMessage(message)) => {
      if matched || message.data.as_ref() != vec![0x5cu8;1200].as_slice() { return Err("payload mismatch".into()); }
      matched=true;
     }
     Some(DataChannelEvent::OnClose) | None => return Err("channel closed before finish".into()),
     _ => {}
    }
   }
   if matched && sent && !notified { notified=true;emit(json!({"event":"payload-matched","bytes":1200})); }
  }
  let sctp = pc.sctp().await.ok_or("sctp missing")?;
  let dtls = sctp.transport();
  let ice = dtls.ice_transport();
  let pair = ice.get_selected_candidate_pair().await?.ok_or("selected pair absent")?;
  let report = json!({"library":"webrtc 0.21.0","features":["runtime-tokio","crypto-ring"],"scope":"physical RTC datagram interoperability, no MIO admission/member/replica qualification","channel":{"id":dc.id(),"label":dc.label().await?,"protocol":dc.protocol().await?,"ordered":dc.ordered().await?,"maxRetransmits":dc.max_retransmits().await?,"negotiated":dc.negotiated().await?},"ice":format!("{:?}",ice.state().await?),"dtls":format!("{:?}",dtls.state().await?),"sctp":format!("{:?}",sctp.state().await?),"selectedPair":{"local":pair.local(),"remote":pair.remote()},"sentBytes":1200,"matchedBytes":1200,"events":events.lock().unwrap().clone()});
  dc.close().await?;
  Ok::<_,Box<dyn std::error::Error+Send+Sync>>(report)
 }.await;
 let closed = pc.close().await;
 emit(json!({"event":"complete","passed":result.is_ok()&&closed.is_ok(),"report":result.as_ref().ok(),"failure":result.as_ref().err().map(ToString::to_string),"pcClosed":closed.is_ok(),"events":events.lock().unwrap().clone()}));
 result?; closed?;
 Ok(())
}
