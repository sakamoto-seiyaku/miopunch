//! One-hop signaling over existing admitted member connections. The envelope
//! carries public delivery tokens and opaque QUIC datagrams, never application
//! streams. The ordinary endpoint gate owns the inner connection's admission.

use crate::admission_gate::{AdmissionGate, ProtocolScope};
use iroh::{
    EndpointId,
    endpoint::{
        Connection, RecvStream, SendStream,
        transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit},
    },
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio::sync::watch;

#[cfg(feature = "member-signaling-test-hooks")]
use std::sync::atomic::{AtomicBool, Ordering};

pub const TRANSPORT_ID: u64 = 0x6d696f6d656d; // "miomem"
const MAGIC: &[u8; 4] = b"MIM1";
const MAX_DATAGRAM: usize = 2048;
const QUEUE_DEPTH: usize = 64;
const MAX_LINKS: usize = 16;
type Route = [u8; 16];

fn address(via: EndpointId, route: Route) -> CustomAddr {
    let mut bytes = [0; 48];
    bytes[..32].copy_from_slice(via.as_bytes());
    bytes[32..].copy_from_slice(&route);
    CustomAddr::from_parts(TRANSPORT_ID, &bytes)
}
fn route(addr: &CustomAddr, via: EndpointId) -> io::Result<Route> {
    if addr.id() != TRANSPORT_ID || addr.data().len() != 48 || &addr.data()[..32] != via.as_bytes()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong member signaling route",
        ));
    }
    Ok(addr.data()[32..].try_into().unwrap())
}
fn check_member(gate: &AdmissionGate, connection: &Connection) -> io::Result<()> {
    if !gate.allows(connection, ProtocolScope::Member) || connection.close_reason().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "signaling requires a live admitted member connection",
        ));
    }
    Ok(())
}
fn check_lifetime(lifetime: Duration) -> io::Result<()> {
    if lifetime.is_zero() || lifetime > Duration::from_secs(300) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "signaling lease must be greater than zero and at most 300 seconds",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct Packet {
    peer: Route,
    bytes: Vec<u8>,
}

async fn read_packet(recv: &mut RecvStream) -> io::Result<Packet> {
    let mut header = [0; 18];
    recv.read_exact(&mut header)
        .await
        .map_err(io::Error::other)?;
    let len = u16::from_be_bytes(header[16..].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_DATAGRAM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid member datagram size",
        ));
    }
    let mut bytes = vec![0; len];
    recv.read_exact(&mut bytes)
        .await
        .map_err(io::Error::other)?;
    Ok(Packet {
        peer: header[..16].try_into().unwrap(),
        bytes,
    })
}
async fn write_packet(send: &mut SendStream, packet: Packet) -> io::Result<()> {
    send.write_all(&packet.peer).await?;
    send.write_all(&(packet.bytes.len() as u16).to_be_bytes())
        .await?;
    send.write_all(&packet.bytes).await?;
    Ok(())
}

async fn transmit_packets(
    connection: &Connection,
    gate: &AdmissionGate,
    send: &mut SendStream,
    queue: &mut mpsc::Receiver<Packet>,
) -> io::Result<()> {
    while let Some(packet) = queue.recv().await {
        check_member(gate, connection)?;
        write_packet(send, packet).await?;
    }
    Ok(())
}

/// Owns only a reserved signaling stream. Expiry/drop interrupts forwarding,
/// without closing the underlying A-B or B-C member connection.
#[derive(Debug)]
pub struct SignalingLease(tokio::task::JoinHandle<()>);
impl Drop for SignalingLease {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl SignalingLease {
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

#[derive(Debug)]
struct ForwardLink {
    connection: Connection,
    sender: mpsc::Sender<Packet>,
    marker: Arc<()>,
}
#[derive(Clone, Debug)]
pub struct MemberForwarder {
    gate: AdmissionGate,
    links: Arc<Mutex<HashMap<Route, ForwardLink>>>,
    #[cfg(feature = "member-signaling-test-hooks")]
    faults: Arc<ForwardFaults>,
}
#[cfg(feature = "member-signaling-test-hooks")]
#[derive(Debug, Default)]
struct ForwardFaults {
    duplicate: AtomicBool,
    observed: Mutex<Vec<Vec<u8>>>,
}
impl MemberForwarder {
    pub fn new(gate: AdmissionGate) -> Self {
        Self {
            gate,
            links: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "member-signaling-test-hooks")]
            faults: Arc::new(ForwardFaults::default()),
        }
    }

    /// Current bounded registrations, including leases whose streams are idle.
    pub fn active_links(&self) -> usize {
        self.links.lock().unwrap().len()
    }

    /// Fault injection at the actual opaque carrier boundary, without mocking
    /// TLS, admission, candidate coordination or QUIC duplicate handling.
    #[cfg(feature = "member-signaling-test-hooks")]
    pub fn test_duplicate_datagrams(&self, enabled: bool) {
        self.faults.duplicate.store(enabled, Ordering::Relaxed);
    }

    /// First 64 actual datagrams observed by B, bounded to 128 KiB. Test-only;
    /// normal builds retain no packet capture or decrypted inner state.
    #[cfg(feature = "member-signaling-test-hooks")]
    pub fn test_observed_datagrams(&self) -> Vec<Vec<u8>> {
        self.faults.observed.lock().unwrap().clone()
    }

    /// The caller dispatches the next reserved bi-stream on its existing
    /// member connection. No data-relay permission is granted by this service.
    pub async fn accept_link(
        &self,
        connection: &Connection,
        lifetime: Duration,
    ) -> io::Result<SignalingLease> {
        check_lifetime(lifetime)?;
        check_member(&self.gate, connection)?;
        let (mut send, mut recv) =
            tokio::time::timeout(Duration::from_secs(5), connection.accept_bi())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "member signaling stream timeout")
                })??;
        let mut hello = [0; 20];
        tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut hello))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "member signaling hello timeout"))?
            .map_err(io::Error::other)?;
        if &hello[..4] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid signaling hello",
            ));
        }
        let own: Route = hello[4..].try_into().unwrap();
        let (sender, mut queue) = mpsc::channel(QUEUE_DEPTH);
        let marker = Arc::new(());
        {
            check_member(&self.gate, connection)?;
            let mut links = self.links.lock().unwrap();
            if links.len() >= MAX_LINKS
                || links.contains_key(&own)
                || links
                    .values()
                    .any(|link| link.connection.remote_id() == connection.remote_id())
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "member signaling link limit or duplicate",
                ));
            }
            links.insert(
                own,
                ForwardLink {
                    connection: connection.clone(),
                    sender,
                    marker: marker.clone(),
                },
            );
        }
        let registration = Registration {
            links: self.links.clone(),
            route: own,
            marker,
        };
        tokio::time::timeout(Duration::from_secs(5), send.write_all(MAGIC))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "member signaling reply timeout")
            })??;
        let gate = self.gate.clone();
        let connection = connection.clone();
        let links = self.links.clone();
        #[cfg(feature = "member-signaling-test-hooks")]
        let faults = self.faults.clone();
        Ok(SignalingLease(tokio::spawn(async move {
            let _registration = registration;
            let receive = async {
                loop {
                    let packet = read_packet(&mut recv).await?;
                    check_member(&gate, &connection)?;
                    if packet.peer == own {
                        continue;
                    }
                    let links = links.lock().unwrap();
                    if let Some(target) = links.get(&packet.peer) {
                        check_member(&gate, &target.connection)?;
                        #[cfg(feature = "member-signaling-test-hooks")]
                        {
                            let mut observed = faults.observed.lock().unwrap();
                            if observed.len() < 64 {
                                observed.push(packet.bytes.clone());
                            }
                            if faults.duplicate.load(Ordering::Relaxed) {
                                let _ = target.sender.try_send(Packet {
                                    peer: own,
                                    bytes: packet.bytes.clone(),
                                });
                            }
                        }
                        // Full target queues drop packets; QUIC owns retransmission.
                        let _ = target.sender.try_send(Packet {
                            peer: own,
                            bytes: packet.bytes,
                        });
                    }
                }
                #[allow(unreachable_code)]
                Ok::<_, io::Error>(())
            };
            let transmit = transmit_packets(&connection, &gate, &mut send, &mut queue);
            let operation = async {
                tokio::select! { result = receive => result, result = transmit => result }
            };
            let _ = tokio::time::timeout(lifetime, operation).await;
        })))
    }
}
struct Registration {
    links: Arc<Mutex<HashMap<Route, ForwardLink>>>,
    route: Route,
    marker: Arc<()>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut links = self.links.lock().unwrap();
        if links
            .get(&self.route)
            .is_some_and(|link| Arc::ptr_eq(&link.marker, &self.marker))
        {
            links.remove(&self.route);
        }
    }
}

#[derive(Debug)]
struct Binding {
    incoming: mpsc::Sender<Packet>,
    outbound: Option<mpsc::Receiver<Packet>>,
}
/// Bind this adapter on the endpoint, then attach an existing member link to
/// `via`. Both ends use the same ordinary connect/accept interface as MQTT.
#[derive(Debug)]
pub struct MemberSignalingTransport {
    via: EndpointId,
    own: Route,
    gate: AdmissionGate,
    binding: Mutex<Option<Binding>>,
    addresses: Watchable<Vec<CustomAddr>>,
    stopped: watch::Sender<bool>,
}
impl MemberSignalingTransport {
    pub fn new(via: EndpointId, gate: AdmissionGate) -> Self {
        Self {
            via,
            own: rand::random(),
            gate,
            binding: Mutex::new(None),
            addresses: Watchable::new(Vec::new()),
            stopped: watch::channel(false).0,
        }
    }
    pub fn address(&self) -> io::Result<CustomAddr> {
        self.addresses.get().first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "member signaling not attached")
        })
    }
    pub async fn attach(
        &self,
        connection: &Connection,
        lifetime: Duration,
    ) -> io::Result<SignalingLease> {
        check_lifetime(lifetime)?;
        check_member(&self.gate, connection)?;
        if connection.remote_id() != self.via {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrong forwarding member",
            ));
        }
        let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(5), async {
            let (mut send, mut recv) = connection.open_bi().await?;
            let mut hello = MAGIC.to_vec();
            hello.extend_from_slice(&self.own);
            send.write_all(&hello).await?;
            let mut ack = [0; 4];
            recv.read_exact(&mut ack).await.map_err(io::Error::other)?;
            if &ack != MAGIC {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid signaling acknowledgement",
                ));
            }
            Ok::<_, io::Error>((send, recv))
        })
        .await
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "member signaling attach timeout")
        })??;
        let (incoming, mut outbound) = {
            let mut binding = self.binding.lock().unwrap();
            let binding = binding.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "member transport not bound")
            })?;
            if *self.stopped.borrow() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "member endpoint stopped",
                ));
            }
            check_member(&self.gate, connection)?;
            (
                binding.incoming.clone(),
                binding.outbound.take().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "member transport already attached",
                    )
                })?,
            )
        };
        let gate = self.gate.clone();
        let connection = connection.clone();
        let addresses = self.addresses.clone();
        let mut stopped = self.stopped.subscribe();
        addresses.set(vec![address(self.via, self.own)]).ok();
        Ok(SignalingLease(tokio::spawn(async move {
            let _advertisement = Advertisement(addresses);
            let receive = async {
                loop {
                    let packet = read_packet(&mut recv).await?;
                    check_member(&gate, &connection)?;
                    let _ = incoming.try_send(packet);
                }
                #[allow(unreachable_code)]
                Ok::<_, io::Error>(())
            };
            let transmit = transmit_packets(&connection, &gate, &mut send, &mut outbound);
            let operation = async {
                tokio::select! { result = receive => result, result = transmit => result, _ = stopped.changed() => Ok(()) }
            };
            let _ = tokio::time::timeout(lifetime, operation).await;
        })))
    }
}
struct Advertisement(Watchable<Vec<CustomAddr>>);
impl Drop for Advertisement {
    fn drop(&mut self) {
        self.0.set(vec![]).ok();
    }
}

#[derive(Debug)]
struct MemberEndpoint {
    via: EndpointId,
    incoming: mpsc::Receiver<Packet>,
    outbound: mpsc::Sender<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    stopped: watch::Sender<bool>,
}
impl Drop for MemberEndpoint {
    fn drop(&mut self) {
        self.stopped.send_replace(true);
        self.addresses.set(vec![]).ok();
    }
}
impl CustomTransport for MemberSignalingTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let mut binding = self.binding.lock().unwrap();
        if binding.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "member transport already bound",
            ));
        }
        let (incoming, receiver) = mpsc::channel(QUEUE_DEPTH);
        let (outbound, queue) = mpsc::channel(QUEUE_DEPTH);
        *binding = Some(Binding {
            incoming,
            outbound: Some(queue),
        });
        Ok(Box::new(MemberEndpoint {
            via: self.via,
            incoming: receiver,
            outbound,
            addresses: self.addresses.clone(),
            stopped: self.stopped.clone(),
        }))
    }
}
impl CustomEndpoint for MemberEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.addresses.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(MemberSender {
            via: self.via,
            outbound: self.outbound.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            match self.incoming.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "member transport stopped",
                    )));
                }
                Poll::Ready(Some(packet)) if packet.bytes.len() <= bufs[0].len() => {
                    bufs[0][..packet.bytes.len()].copy_from_slice(&packet.bytes);
                    metas[0].len = packet.bytes.len();
                    metas[0].stride = packet.bytes.len();
                    infos[0] = RecvInfo::new(
                        address(self.via, packet.peer),
                        self.addresses.get().first().cloned(),
                    );
                    return Poll::Ready(Ok(1));
                }
                _ => {}
            }
        }
    }
}
#[derive(Debug)]
struct MemberSender {
    via: EndpointId,
    outbound: mpsc::Sender<Packet>,
}
impl CustomSender for MemberSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        route(addr, self.via).is_ok()
    }
    fn poll_send(
        &self,
        _cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let peer = match route(dst, self.via) {
            Ok(peer) => peer,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if transmit.contents.is_empty() || transmit.contents.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid member datagram size",
            )));
        }
        let result = match self.outbound.try_send(Packet {
            peer,
            bytes: transmit.contents.to_vec(),
        }) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "member signaling stopped",
            )),
        };
        Poll::Ready(result)
    }
}
