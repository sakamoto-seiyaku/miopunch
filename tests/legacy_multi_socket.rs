//! Physical UDP bindings on the original authenticated QUIC connection.
#[path = "support/control_handoff.rs"]
mod handoff;
use std::{net::Ipv4Addr, sync::Arc, time::Duration};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{
        presets,
        transports::{ConnectionPathSelector, FourTuple, PathSelection, PathSelectionContext},
    },
};
use iroh_base::{SecretKey, TransportAddr};
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, CONTROL_ALPN},
    admission_proof::{AdmissionScope, AdmissionSecret},
    control_session::ControlSession,
    tcp_transport::{TcpTransport, Traffic},
};

#[derive(Debug)]
struct IpThenControl;
impl ConnectionPathSelector for IpThenControl {
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let mut selected = PathSelection::none();
        if let Some(path) = context
            .paths()
            .find(|p| matches!(p.network_path(), FourTuple::Ip { .. }) && context.is_current(p))
            .or_else(|| {
                context
                    .paths()
                    .find(|p| matches!(p.network_path(), FourTuple::Ip { .. }))
            })
            .or_else(|| context.paths().next())
        {
            selected.set(&path);
        }
        selected
    }
}

struct Pair {
    c: iroh::endpoint::RestrictedEndpoint,
    v: iroh::endpoint::RestrictedEndpoint,
    pending: Option<
        std::sync::Mutex<Vec<tokio::sync::mpsc::UnboundedReceiver<handoff::PendingControl>>>,
    >,
    c_addr: iroh_base::CustomAddr,
}
impl Pair {
    async fn new() -> Self {
        Self::build(false).await
    }
    async fn new_pending() -> Self {
        Self::build(true).await
    }
    async fn build(pending: bool) -> Self {
        Self::build_with_selector(pending, Arc::new(IpThenControl)).await
    }
    async fn build_with_selector(pending: bool, selector: Arc<dyn ConnectionPathSelector>) -> Self {
        let keys = [SecretKey::generate(), SecretKey::generate()];
        let members = keys
            .iter()
            .map(|k| *k.public().as_bytes())
            .collect::<Vec<_>>();
        let mut endpoints = Vec::new();
        let mut addresses = Vec::new();
        let mut receivers = Vec::new();
        for key in keys {
            let gate = AdmissionGate::new(
                key.public(),
                AdmissionScope::new([61; 32], 1),
                Some(AdmissionSecret::from_saved([61; 32])),
            );
            gate.update_authorization(members.clone(), []);
            let carrier = Arc::new(TcpTransport::new(
                Ipv4Addr::LOCALHOST,
                Arc::new(Traffic::default()),
            ));
            let builder = Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .secret_key(key)
                .alpns(vec![CONTROL_ALPN.to_vec()])
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .clear_relay_transports()
                .add_custom_transport(carrier.clone())
                .connection_path_selector(selector.clone());
            let builder = if pending {
                let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                receivers.push(receiver);
                builder.restricted_application(handoff::PendingDataPolicy {
                    gate,
                    pending: sender,
                })
            } else {
                gate.configure(builder)
            };
            let endpoint = builder.bind().await.unwrap();
            endpoints.push(endpoint);
            addresses.push(TcpTransport::address(carrier.bound_addr().unwrap()));
        }
        Self {
            v: endpoints.pop().unwrap(),
            c: endpoints.pop().unwrap(),
            c_addr: addresses.remove(0),
            pending: pending.then(|| std::sync::Mutex::new(receivers)),
        }
    }
    async fn connect(&self) -> (ControlSession, ControlSession) {
        let target =
            EndpointAddr::new(self.c.id()).with_addrs([TransportAddr::Custom(self.c_addr.clone())]);
        if let Some(receivers) = &self.pending {
            let c = self.c.clone();
            let v = self.v.clone();
            let accept = tokio::spawn(async move {
                c.accept()
                    .await
                    .unwrap()
                    .await
                    .map_err(std::io::Error::other)
            });
            let dial = tokio::spawn(async move {
                v.connect(target, CONTROL_ALPN)
                    .await
                    .map_err(std::io::Error::other)
            });
            let mut receivers = receivers.lock().unwrap();
            let c = receivers[0].recv().await.unwrap();
            let v = receivers[1].recv().await.unwrap();
            (c.session(accept), v.session(dial))
        } else {
            let (c, v) = self.connect_connections().await;
            (
                ControlSession::from_control_connection(c),
                ControlSession::from_control_connection(v),
            )
        }
    }
    async fn connect_connections(
        &self,
    ) -> (
        iroh::endpoint::ControlConnection,
        iroh::endpoint::ControlConnection,
    ) {
        let target =
            EndpointAddr::new(self.c.id()).with_addrs([TransportAddr::Custom(self.c_addr.clone())]);
        let (c, v) = tokio::join!(
            async {
                self.c
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            },
            self.v.connect_control(target, CONTROL_ALPN)
        );
        (c.unwrap(), v.unwrap())
    }
}

#[tokio::test]
async fn nonprimary_winner_returns_through_its_physical_socket_on_the_original_connection() {
    winner_connection(Ipv4Addr::LOCALHOST, true).await;
}

#[tokio::test]
async fn authenticated_winner_keeps_its_actual_source_ip_despite_packets_to_another_local_ip() {
    winner_connection(Ipv4Addr::new(127, 0, 0, 2), false).await;
}

async fn winner_connection(receiver_ip: Ipv4Addr, network_change: bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        let pair = Pair::new_pending().await;
        let (c, v) = pair.connect().await;
        let stable = (c.stable_id(), v.stable_id());
        let mut cl = c.register_udp_traversal([61; 16]).await.unwrap();
        let mut vl = v.register_udp_traversal([61; 16]).await.unwrap();
        assert_eq!(cl.listen_random_ports(2).await.unwrap().bound, 2);
        assert_eq!(vl.listen_random_ports(2).await.unwrap().bound, 2);
        let cb = cl.local_sockets()[1];
        let vb = vl.local_sockets()[2];
        let ca = (receiver_ip, cb.local_addr.port()).into();
        let va: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, vb.local_addr.port()).into();
        vl.sender_for(vb.id)
            .unwrap()
            .send(ca, b"request", 0)
            .await
            .unwrap();
        let packet = cl.recv().await.unwrap();
        assert_eq!(packet.local_socket_id, cb.id);
        assert_eq!(packet.local_addr.port(), cb.local_addr.port());
        assert_eq!(packet.local_addr.ip(), std::net::IpAddr::V4(receiver_ip));
        assert_eq!(packet.remote, va);
        cl.sender_for(packet.local_socket_id)
            .unwrap()
            .with_source_ip(packet.local_addr.ip())
            .unwrap()
            .send(packet.remote, b"response", 0)
            .await
            .unwrap();
        let response = vl.recv().await.unwrap();
        assert_eq!(response.local_socket_id, vb.id);
        assert_eq!(response.remote, ca);
        let mut c_winner = cl
            .retain_winner(cb.id, packet.local_addr)
            .await
            .unwrap()
            .unwrap();
        let mut v_winner = vl
            .retain_winner(vb.id, response.local_addr)
            .await
            .unwrap()
            .unwrap();
        v.open_data_path_from(
            iroh::endpoint::LocalSocketBinding {
                id: vb.id,
                local_addr: response.local_addr,
            },
            ca,
        )
        .await
        .unwrap();
        while !c.has_data_path() || !v.has_data_path() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(cl);
        drop(vl);
        let unauthenticated = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        unauthenticated
            .send_to(b"\0unrelated", (Ipv4Addr::LOCALHOST, cb.local_addr.port()))
            .await
            .unwrap();
        let (cd, vd) = tokio::join!(c.commit_data_path(), v.commit_data_path());
        let (cd, vd) = (cd.unwrap(), vd.unwrap());
        assert_eq!(
            (cd.connection().stable_id(), vd.connection().stable_id()),
            stable
        );
        assert_eq!(cd.connection().remote_id(), pair.v.id());
        assert_eq!(vd.connection().remote_id(), pair.c.id());
        assert!(
            cd.connection()
                .paths()
                .iter()
                .any(|p| p.is_selected() && p.local_socket_id() == Some(cb.id))
        );
        assert!(
            vd.connection()
                .paths()
                .iter()
                .any(|p| p.is_selected() && p.local_socket_id() == Some(vb.id))
        );
        // Public address observation remains the actual receiving interface.
        // Socket identity is independent, even for a nonprimary descriptor.
        let c_paths = cd.connection().paths();
        let c_path = c_paths.iter().find(|p| p.is_selected()).unwrap();
        assert_eq!(
            c_path.local_addr(),
            &iroh::endpoint::LocalTransportAddr::Ip(Some(receiver_ip.into()))
        );
        let v_paths = vd.connection().paths();
        let v_path = v_paths.iter().find(|p| p.is_selected()).unwrap();
        assert_eq!(
            v_path.local_addr(),
            &iroh::endpoint::LocalTransportAddr::Ip(Some(Ipv4Addr::LOCALHOST.into()))
        );
        let (mut send, mut recv) = vd.connection().open_bi().await.unwrap();
        send.write_all(b"winner-socket-business").await.unwrap();
        send.finish().unwrap();
        let (mut reply, mut request) = cd.connection().accept_bi().await.unwrap();
        assert_eq!(
            request.read_to_end(64).await.unwrap(),
            b"winner-socket-business"
        );
        reply.write_all(b"same-connection-response").await.unwrap();
        reply.finish().unwrap();
        assert_eq!(
            recv.read_to_end(64).await.unwrap(),
            b"same-connection-response"
        );
        c_winner.commit(cd.connection()).unwrap();
        v_winner.commit(vd.connection()).unwrap();
        drop(c_winner);
        drop(v_winner);
        // A second connection to the same peer must keep its own physical
        // binding even while the peer-level selector prefers this winner.
        let (other_c, other_v) = pair.connect().await;
        let other_stable = (other_c.stable_id(), other_v.stable_id());
        assert_ne!(other_stable, stable);
        other_v
            .open_data_path(pair.c.bound_sockets()[0])
            .await
            .unwrap();
        while !other_c.has_data_path() || !other_v.has_data_path() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let (other_cd, other_vd) =
            tokio::join!(other_c.commit_data_path(), other_v.commit_data_path());
        let (other_cd, other_vd) = (other_cd.unwrap(), other_vd.unwrap());
        assert!(
            !other_cd
                .connection()
                .paths()
                .iter()
                .any(|p| p.local_socket_id() == Some(cb.id))
        );
        assert!(
            !other_vd
                .connection()
                .paths()
                .iter()
                .any(|p| p.local_socket_id() == Some(vb.id))
        );
        let (mut send, _) = other_vd.connection().open_bi().await.unwrap();
        send.write_all(b"second-original-connection").await.unwrap();
        send.finish().unwrap();
        let (_, mut recv) = other_cd.connection().accept_bi().await.unwrap();
        assert_eq!(
            recv.read_to_end(64).await.unwrap(),
            b"second-original-connection"
        );
        assert_eq!(
            (
                other_cd.connection().stable_id(),
                other_vd.connection().stable_id()
            ),
            other_stable
        );
        // Real interface unchanged: a private local token must not be compared
        // directly with the interface IP set or cleared into primary routing.
        if network_change {
            pair.c.network_change().await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (mut followup, _) = vd.connection().open_bi().await.unwrap();
        followup.write_all(b"after-network-change").await.unwrap();
        followup.finish().unwrap();
        let (_, mut followup) = cd.connection().accept_bi().await.unwrap();
        assert_eq!(
            followup.read_to_end(64).await.unwrap(),
            b"after-network-change"
        );
        assert!(
            cd.connection()
                .paths()
                .iter()
                .any(|p| p.is_selected() && p.local_socket_id() == Some(cb.id))
        );
        pair.c.close().await;
        pair.v.close().await;
    })
    .await
    .unwrap();
}

fn assert_port_closed(port: u16) {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).unwrap();
}

#[tokio::test]
async fn interrupted_listener_setup_preserves_actual_partial_costs_and_closes_bindings() {
    let pair = Pair::new().await;
    let (c, _v) = pair.connect().await;
    let mut lease = c.register_udp_traversal([14; 16]).await.unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let result = lease
        .listen_random_ports_checked(256, &|| {
            if checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 3 {
                Err(std::io::Error::other("authorized cancellation"))
            } else {
                Ok(())
            }
        })
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("authorized cancellation")
    );
    let report = lease.listener_report();
    assert_eq!((report.attempted, report.bound, report.failed), (3, 3, 0));
    let bindings = lease.local_sockets();
    assert_eq!(bindings.len(), 4);
    drop(lease);
    for binding in bindings.into_iter().skip(1) {
        assert_port_closed(binding.local_addr.port());
    }
    pair.c.close().await;
    pair.v.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn every_physical_sender_isolates_packet_ttl_and_preserves_source_when_ttl_degrades() {
    use std::io::BufRead;
    let pair = Pair::new().await;
    let (c, _v) = pair.connect().await;
    let mut lease = c.register_udp_traversal([12; 16]).await.unwrap();
    lease.listen_random_ports(1).await.unwrap();
    // A real kernel receiver reports TTL ancillary data for each datagram.
    let mut receiver = std::process::Command::new("python3")
        .args(["-u", "-c", "import socket,struct,json\ns=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.setsockopt(socket.IPPROTO_IP,12,1);s.bind(('127.0.0.1',0));s.settimeout(3);print(s.getsockname()[1],flush=True)\nfor i in range(8):\n d,a,f,r=s.recvmsg(1024,1024);t=[struct.unpack('i',v)[0] for l,k,v in a if l==socket.IPPROTO_IP and k==socket.IP_TTL][0];print(json.dumps({'payload':d[27:].decode(),'source':r,'ttl':t}),flush=True)"])
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .spawn().unwrap();
    let mut output = std::io::BufReader::new(receiver.stdout.take().unwrap());
    let mut port = String::new();
    output.read_line(&mut port).unwrap();
    let target = (Ipv4Addr::LOCALHOST, port.trim().parse::<u16>().unwrap()).into();
    let source = Ipv4Addr::new(127, 0, 0, 2);
    let bindings = lease.local_sockets();
    for binding in &bindings {
        let sender = lease
            .sender_for(binding.id)
            .unwrap()
            .with_source_ip(source.into())
            .unwrap();
        for ttl in [7, 4, 0] {
            assert!(
                !sender
                    .send(
                        target,
                        format!("{}:{ttl}", binding.id.get()).as_bytes(),
                        ttl
                    )
                    .await
                    .unwrap()
            );
        }
    }
    pair.c.test_force_traversal_ttl_unavailable().await;
    let mut fresh = c.register_udp_traversal([13; 16]).await.unwrap();
    fresh.listen_random_ports(1).await.unwrap();
    let new_binding = *fresh.local_sockets().last().unwrap();
    for binding in [bindings[1], new_binding] {
        let sender = lease
            .sender_for(binding.id)
            .or_else(|_| fresh.sender_for(binding.id))
            .unwrap()
            .with_source_ip(source.into())
            .unwrap();
        assert!(
            sender
                .send(
                    target,
                    format!("{}:degraded", binding.id.get()).as_bytes(),
                    7
                )
                .await
                .unwrap()
        );
    }
    let lines = tokio::task::spawn_blocking(move || {
        let lines = output.lines().collect::<Result<Vec<_>, _>>().unwrap();
        let status = receiver.wait().unwrap();
        assert!(status.success());
        lines
    })
    .await
    .unwrap();
    let mut default_ttl = None;
    for line in lines {
        let record: serde_json::Value = serde_json::from_str(&line).unwrap();
        let (id, requested) = record["payload"].as_str().unwrap().split_once(':').unwrap();
        let id = id.parse::<u64>().unwrap();
        let binding = lease
            .local_sockets()
            .into_iter()
            .chain(fresh.local_sockets())
            .find(|b| b.id.get() == id)
            .unwrap();
        assert_eq!(record["source"][0], source.to_string());
        assert_eq!(record["source"][1], binding.local_addr.port());
        let observed = record["ttl"].as_u64().unwrap();
        match requested {
            "7" => assert_eq!(observed, 7),
            "4" => assert_eq!(observed, 4),
            "0" => {
                assert!(observed > 7);
                assert_eq!(*default_ttl.get_or_insert(observed), observed);
            }
            "degraded" => assert_eq!(observed, default_ttl.unwrap()),
            _ => unreachable!(),
        }
        println!(
            "MIO61_TTL local_socket_id={id} source={}:{} requested={requested} observed={observed}",
            source,
            binding.local_addr.port()
        );
    }
    drop((lease, fresh));
    pair.c.close().await;
    pair.v.close().await;
}

fn packet_auth(
    connection: &iroh::endpoint::ControlConnection,
    local: &iroh::endpoint::RestrictedEndpoint,
    remote: &iroh::endpoint::RestrictedEndpoint,
    round: &[u8; 16],
) -> miopunch_native_probe::nat_scheduler::PacketAuth {
    let mut key = [0; 32];
    connection
        .export_keying_material(&mut key, b"miopunch-nat-traversal-v1", round)
        .unwrap();
    miopunch_native_probe::nat_scheduler::PacketAuth {
        key,
        local: *local.id().as_bytes(),
        remote: *remote.id().as_bytes(),
    }
}

fn echo_receipt(
    request: &[u8],
    peer: &miopunch_native_probe::nat_scheduler::PacketAuth,
) -> Vec<u8> {
    let mut body = request[..17].to_vec();
    body[0] = 1;
    let mut mac = blake3::Hasher::new_keyed(&peer.key);
    mac.update(&peer.local);
    mac.update(&peer.remote);
    mac.update(&body);
    body.extend_from_slice(mac.finalize().as_bytes());
    body
}

#[tokio::test]
async fn same_round_authenticated_receipt_cannot_move_to_another_physical_socket() {
    use miopunch_native_probe::{
        nat_mapping::NodeProbeBudget,
        nat_scheduler::{self, PhasePlan},
        nat_selector::{Behavior, Role},
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let pair = Pair::new().await;
        let (c, v) = pair.connect_connections().await;
        let round = [10; 16];
        let mut cl = c.register_udp_traversal(round).await.unwrap();
        let mut vl = v.register_udp_traversal(round).await.unwrap();
        cl.listen_random_ports(2).await.unwrap();
        let bindings = cl.local_sockets();
        let ca = packet_auth(&c, &pair.c, &pair.v, &round);
        let va = packet_auth(&v, &pair.v, &pair.c, &round);
        assert_eq!(ca.key, va.key);
        let phase = PhasePlan::new(
            Behavior {
                role: Role::Sender,
                ttl: 0,
                send_delay_ms: 0,
                ports_range: 0,
                ports_random: 0,
                listen_random: 2,
            },
            0,
            &pair.v.bound_sockets(),
            &[],
            0,
        );
        let budget = NodeProbeBudget::new(0, 20);
        let injector = async {
            let mut source = None;
            for _ in 0..3 {
                let packet = vl.recv().await.unwrap();
                if packet.remote.port() == bindings[1].local_addr.port() {
                    source = Some(packet);
                }
            }
            let packet = source.unwrap();
            let receipt = echo_receipt(&packet.payload, &va);
            vl.sender()
                .send(
                    (Ipv4Addr::LOCALHOST, bindings[2].local_addr.port()).into(),
                    &receipt,
                    0,
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            vl.sender().send(packet.remote, &receipt, 0).await.unwrap();
        };
        let (result, ()) = tokio::join!(
            nat_scheduler::run(&phase, &mut cl, &ca, &budget, &|| Ok(())),
            injector
        );
        let (winner, trace) = result.unwrap();
        assert_eq!(winner.local_socket_id, bindings[1].id);
        assert_eq!(trace.rejected_receipts, 1);
        assert!(winner.response);
        pair.c.close().await;
        pair.v.close().await;
    })
    .await
    .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_real_target_send_error_does_not_truncate_following_candidates_or_random_work() {
    use miopunch_native_probe::{
        nat_mapping::NodeProbeBudget,
        nat_scheduler::{self, PhasePlan},
        nat_selector::{Behavior, Role},
    };
    let pair = Pair::new().await;
    let (c, v) = pair.connect_connections().await;
    let round = [11; 16];
    let mut cl = c.register_udp_traversal(round).await.unwrap();
    let mut vl = v.register_udp_traversal(round).await.unwrap();
    let ca = packet_auth(&c, &pair.c, &pair.v, &round);
    let va = packet_auth(&v, &pair.v, &pair.c, &round);
    let sender = PhasePlan::new(
        Behavior {
            role: Role::Sender,
            ttl: 7,
            send_delay_ms: 0,
            ports_range: 0,
            ports_random: 1000,
            listen_random: 0,
        },
        0,
        &pair.v.bound_sockets(),
        &["255.255.255.255:9".parse().unwrap()],
        0,
    );
    let receiver = PhasePlan::new(
        Behavior {
            role: Role::Receiver,
            ttl: 7,
            send_delay_ms: 500,
            ports_range: 0,
            ports_random: 0,
            listen_random: 0,
        },
        0,
        &pair.c.bound_sockets(),
        &[],
        0,
    );
    let cb = NodeProbeBudget::new(0, 1000);
    let vb = NodeProbeBudget::new(0, 3);
    let (cs, vs) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(
            nat_scheduler::run(&sender, &mut cl, &ca, &cb, &|| Ok(())),
            nat_scheduler::run(&receiver, &mut vl, &va, &vb, &|| Ok(()))
        )
    })
    .await
    .unwrap();
    let (winner, trace) = cs.unwrap();
    assert!(winner.response);
    assert_eq!(trace.send_errors, 1);
    assert!(trace.probes > 0);
    assert!(trace.random_iterations > 0);
    assert!(trace.send_attempts >= trace.probes + 1);
    assert!(vs.unwrap().0.before_probe);
    pair.c.close().await;
    pair.v.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn os_bind_failures_keep_exact_attempt_count_and_release_partial_pool() {
    if std::env::var_os("MIO61_RESOURCE_CHILD").is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "os_bind_failures_keep_exact_attempt_count_and_release_partial_pool",
                "--nocapture",
            ])
            .env("MIO61_RESOURCE_CHILD", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut peak = 0;
        while child.try_wait().unwrap().is_none() {
            if let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
                peak = peak.max(fds.count());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        println!("MIO61_PARTIAL_OS_FD_PEAK value={peak}\n{stdout}");
        assert!(stdout.contains("MIO61_PARTIAL_BIND attempted=256"));
        return;
    }
    let pair = Pair::new().await;
    let (c, _v) = pair.connect().await;
    let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let mut limit = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    limit.current = Some(before as u64 + 8);
    rustix::process::setrlimit(rustix::process::Resource::Nofile, limit).unwrap();
    let mut lease = c.register_udp_traversal([8; 16]).await.unwrap();
    let report = lease.listen_random_ports(256).await.unwrap();
    assert_eq!(report.attempted, 256);
    assert!(report.bound > 0 && report.bound < 256);
    assert_eq!(report.bound + report.failed, 256);
    println!(
        "MIO61_PARTIAL_BIND attempted=256 bound={} failed={} soft_limit={}",
        report.bound,
        report.failed,
        limit.current.unwrap()
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ports = lease
        .local_sockets()
        .into_iter()
        .skip(1)
        .map(|b| b.local_addr.port())
        .collect::<Vec<_>>();
    drop(lease);
    for port in ports {
        assert_port_closed(port);
    }
    assert!(std::fs::read_dir("/proc/self/fd").unwrap().count() <= before + 2);
    pair.c.close().await;
    pair.v.close().await;
}

#[tokio::test]
async fn discarded_winner_paths_release_descriptors_while_original_control_connection_lives() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let pair = Pair::new().await;
        let (c, v) = pair.connect_connections().await;
        let stable = (c.stable_id(), v.stable_id());
        let mut cl = c.register_udp_traversal([9; 16]).await.unwrap();
        let mut vl = v.register_udp_traversal([9; 16]).await.unwrap();
        cl.listen_random_ports(2).await.unwrap();
        vl.listen_random_ports(2).await.unwrap();
        let cb = cl.local_sockets()[1];
        let vb = vl.local_sockets()[2];
        let ca = std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, cb.local_addr.port()));
        vl.sender_for(vb.id)
            .unwrap()
            .send(ca, b"hint", 0)
            .await
            .unwrap();
        let packet = cl.recv().await.unwrap();
        cl.sender_for(cb.id)
            .unwrap()
            .with_source_ip(packet.local_addr.ip())
            .unwrap()
            .send(packet.remote, b"return", 0)
            .await
            .unwrap();
        let response = vl.recv().await.unwrap();
        let cw = cl
            .retain_winner(cb.id, packet.local_addr)
            .await
            .unwrap()
            .unwrap();
        let vw = vl
            .retain_winner(vb.id, response.local_addr)
            .await
            .unwrap()
            .unwrap();
        v.open_data_path_from(
            iroh::endpoint::LocalSocketBinding {
                id: vb.id,
                local_addr: response.local_addr,
            },
            ca,
        )
        .await
        .unwrap();
        while !v.paths().iter().any(|p| p.local_socket_id() == Some(vb.id))
            || !c.paths().iter().any(|p| p.local_socket_id() == Some(cb.id))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // This is still a restricted Control connection: the validated IP path
        // is a backup, and the original TCP control path remains selected.
        // Keep provisional guards alive while testing protocol path disposal.
        drop((cl, vl));
        v.paths()
            .iter()
            .find(|p| p.local_socket_id() == Some(vb.id))
            .unwrap()
            .close()
            .unwrap();
        // Noq retains the descriptor during abandonment grace and releases it
        // only after the last associated path is actually Discarded.
        loop {
            if std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, vb.local_addr.port())).is_ok()
                && std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, cb.local_addr.port())).is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!((c.stable_id(), v.stable_id()), stable);
        assert!(c.close_reason().is_none() && v.close_reason().is_none());
        let exchange = v.open_control_exchange().await.unwrap();
        exchange
            .send_message(
                iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 },
                b"original-control-alive",
            )
            .await
            .unwrap();
        let exchange = c.accept_control_exchange().await.unwrap();
        assert_eq!(
            exchange
                .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 })
                .await
                .unwrap(),
            b"original-control-alive"
        );
        drop((cw, vw));
        pair.c.close().await;
        pair.v.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn all_256_physical_listeners_close_on_cancel_and_rounds_do_not_accumulate() {
    let pair = Pair::new().await;
    let (c, _v) = pair.connect().await;
    for round in [1, 2] {
        #[cfg(target_os = "linux")]
        let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
        let mut lease = c.register_udp_traversal([round; 16]).await.unwrap();
        let report = lease.listen_random_ports(256).await.unwrap();
        assert_eq!(
            (report.attempted, report.bound, report.failed),
            (256, 256, 0)
        );
        let bindings = lease.local_sockets();
        #[cfg(target_os = "linux")]
        let peak = std::fs::read_dir("/proc/self/fd").unwrap().count();
        assert_eq!(bindings.len(), 257);
        assert_eq!(bindings.iter().filter(|b| b.id.is_primary()).count(), 1);
        let ports = bindings
            .iter()
            .skip(1)
            .map(|b| b.local_addr.port())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ports.len(), 256);
        for port in &ports {
            assert!(std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, *port)).is_err());
        }
        let old_sender = lease.sender_for(bindings[1].id).unwrap();
        drop(lease);
        for port in ports {
            assert_port_closed(port);
        }
        assert!(
            old_sender
                .send(pair.v.bound_sockets()[0], b"late", 0)
                .await
                .unwrap_err()
                .to_string()
                .contains("round closed")
        );
        assert!(std::net::UdpSocket::bind(pair.c.bound_sockets()[0]).is_err());
        #[cfg(target_os = "linux")]
        {
            let after = std::fs::read_dir("/proc/self/fd").unwrap().count();
            println!(
                "MIO61_RESOURCE round={round} bound=256 fd_before={before} fd_peak={peak} fd_after={after}"
            );
            assert!(peak >= before + 256);
            assert!(after <= before + 2);
        }
    }
    pair.c.close().await;
    pair.v.close().await;
}

#[tokio::test]
async fn a_provisional_winner_closes_when_cancelled_and_cannot_be_used_by_another_connection() {
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let (foreign_c, foreign_v) = pair.connect().await;
    assert_ne!(foreign_c.stable_id(), c.stable_id());
    let mut lease = v.register_udp_traversal([3; 16]).await.unwrap();
    lease.listen_random_ports(2).await.unwrap();
    let bindings = lease.local_sockets();
    let actual = std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, bindings[2].local_addr.port()));
    let winner = lease
        .retain_winner(bindings[2].id, actual)
        .await
        .unwrap()
        .unwrap();
    assert_port_closed(bindings[1].local_addr.port());
    assert!(
        foreign_v
            .open_data_path_from(
                iroh::endpoint::LocalSocketBinding {
                    id: bindings[2].id,
                    local_addr: actual
                },
                pair.c.bound_sockets()[0]
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
    drop(lease);
    assert!(
        std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, bindings[2].local_addr.port())).is_err()
    );
    drop(winner);
    assert_port_closed(bindings[2].local_addr.port());
    drop(foreign_c);
    drop(foreign_v);
    drop(c);
    pair.c.close().await;
    pair.v.close().await;
}

#[tokio::test]
async fn cross_socket_round_packets_are_ignored_and_network_change_invalidates_pending_bindings() {
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let mut intended = c.register_udp_traversal([4; 16]).await.unwrap();
    let mut foreign = c.register_udp_traversal([5; 16]).await.unwrap();
    let remote = v.register_udp_traversal([4; 16]).await.unwrap();
    intended.listen_random_ports(2).await.unwrap();
    foreign.listen_random_ports(2).await.unwrap();
    let wrong = foreign.local_sockets()[1];
    remote
        .sender()
        .send(
            (Ipv4Addr::LOCALHOST, wrong.local_addr.port()).into(),
            b"cross-socket",
            0,
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), intended.recv())
            .await
            .is_err()
    );
    let pending = intended.local_sockets()[1];
    let old = intended.sender_for(pending.id).unwrap();
    pair.c.network_change().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), intended.recv())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        old.send(pair.v.bound_sockets()[0], b"old-generation", 0)
            .await
            .is_err()
    );
    assert_port_closed(pending.local_addr.port());
    assert_port_closed(wrong.local_addr.port());
    // Dropping a stale generation cannot unregister a fresh lease of the same
    // opaque round, nor can its old sender be revived by that registration.
    let mut fresh = c.register_udp_traversal([4; 16]).await.unwrap();
    drop(intended);
    remote
        .sender()
        .send(pair.c.bound_sockets()[0], b"fresh", 0)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), fresh.recv())
            .await
            .unwrap()
            .unwrap()
            .payload,
        b"fresh"
    );
    pair.c.close().await;
    pair.v.close().await;
}

#[derive(Debug, Default)]
struct ExactSocketSelector {
    target: std::sync::atomic::AtomicUsize,
    switch_to: std::sync::Mutex<Option<iroh::endpoint::LocalSocketId>>,
    saved: std::sync::Mutex<Option<PathSelection>>,
    replay_stale: std::sync::atomic::AtomicBool,
    stale_callbacks: std::sync::atomic::AtomicUsize,
    same_address_observations: std::sync::Mutex<Vec<(usize, usize)>>,
}
impl ConnectionPathSelector for ExactSocketSelector {
    fn reevaluation_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        use std::sync::atomic::Ordering::SeqCst;
        let paths: Vec<_> = context.paths().collect();
        let target = context.connection_id() == Some(self.target.load(SeqCst));
        if target {
            let address_matches = paths
                .iter()
                .filter(|p| context.current() == Some(p.network_path()))
                .count();
            let exact_matches = paths.iter().filter(|p| context.is_current(p)).count();
            if address_matches > 1 {
                self.same_address_observations
                    .lock()
                    .unwrap()
                    .push((address_matches, exact_matches));
            }
            if self.replay_stale.load(SeqCst) {
                self.stale_callbacks.fetch_add(1, SeqCst);
                return self.saved.lock().unwrap().clone().unwrap();
            }
        }
        let requested = *self.switch_to.lock().unwrap();
        let selected = paths
            .iter()
            .find(|p| target && requested.is_some() && p.local_socket_id() == requested)
            .or_else(|| {
                paths
                    .iter()
                    .find(|p| p.network_path().is_ip() && context.is_current(p))
            })
            .or_else(|| paths.iter().find(|p| p.network_path().is_ip()))
            .or_else(|| paths.first());
        let mut selection = PathSelection::none();
        if let Some(path) = selected {
            selection.set(path);
            if target && path.local_socket_id().is_some() {
                let mut saved = self.saved.lock().unwrap();
                if saved.is_none() {
                    *saved = Some(selection.clone());
                }
            }
        }
        selection
    }
}

#[tokio::test]
async fn same_address_sockets_keep_exact_current_and_reject_retired_selection() {
    tokio::time::timeout(Duration::from_secs(12), exact_socket_lifecycle())
        .await
        .unwrap();
}

async fn exact_socket_lifecycle() {
    use std::sync::atomic::Ordering::SeqCst;
    let selector = Arc::new(ExactSocketSelector::default());
    let pair = Pair::build_with_selector(true, selector.clone()).await;
    // Pair dials from v to c; operate on the dialing original.
    let (v, c) = pair.connect().await;
    let stable = (c.stable_id(), v.stable_id());
    selector.target.store(c.stable_id(), SeqCst);
    // Independent retained rounds give this original connection two
    // descriptors with the same local IP and the same remote IP:port.
    let mut first = c.register_udp_traversal([101; 16]).await.unwrap();
    let mut second = c.register_udp_traversal([102; 16]).await.unwrap();
    first.listen_random_ports(1).await.unwrap();
    second.listen_random_ports(1).await.unwrap();
    let a = first.local_sockets()[1];
    let b = second.local_sockets()[1];
    assert_ne!(a.id, b.id);
    let a_addr = (Ipv4Addr::LOCALHOST, a.local_addr.port()).into();
    let b_addr = (Ipv4Addr::LOCALHOST, b.local_addr.port()).into();
    let a_winner = first.retain_winner(a.id, a_addr).await.unwrap().unwrap();
    let mut b_winner = second.retain_winner(b.id, b_addr).await.unwrap().unwrap();
    let remote = pair.c.bound_sockets()[0];
    c.open_data_path_from(
        iroh::endpoint::LocalSocketBinding {
            id: a.id,
            local_addr: a_addr,
        },
        remote,
    )
    .await
    .unwrap();
    while !c
        .control_handle()
        .paths()
        .iter()
        .any(|p| p.is_selected() && p.local_socket_id() == Some(a.id))
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    *selector.switch_to.lock().unwrap() = Some(b.id);
    c.open_data_path_from(
        iroh::endpoint::LocalSocketBinding {
            id: b.id,
            local_addr: b_addr,
        },
        remote,
    )
    .await
    .unwrap();
    while !c
        .control_handle()
        .paths()
        .iter()
        .any(|p| p.is_selected() && p.local_socket_id() == Some(b.id))
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let observations = selector.same_address_observations.lock().unwrap().clone();
    assert!(
        observations
            .iter()
            .any(|&(old, exact)| old == 2 && exact == 1),
        "{observations:?}"
    );
    if let Some(path) = c
        .control_handle()
        .paths()
        .iter()
        .find(|p| p.local_socket_id() == Some(a.id))
    {
        path.close().unwrap();
    }
    drop(a_winner);
    drop(first);
    drop(second);
    assert_port_closed(a.local_addr.port());
    selector.replay_stale.store(true, SeqCst);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(selector.stale_callbacks.load(SeqCst) > 0);
    assert!(
        !c.control_handle()
            .paths()
            .iter()
            .any(|p| p.local_socket_id() == Some(a.id))
    );
    assert!(
        c.control_handle()
            .paths()
            .iter()
            .any(|p| p.is_selected() && p.local_socket_id() == Some(b.id))
    );
    let (cd, vd) = tokio::join!(c.commit_data_path(), v.commit_data_path());
    let (cd, vd) = (cd.unwrap(), vd.unwrap());
    assert_eq!(
        (cd.connection().stable_id(), vd.connection().stable_id()),
        stable
    );
    b_winner.commit(cd.connection()).unwrap();
    drop(b_winner);
    let (mut send, mut recv) = cd.connection().open_bi().await.unwrap();
    send.write_all(b"same-address-exact-socket").await.unwrap();
    send.finish().unwrap();
    let (mut send, mut request) = vd.connection().accept_bi().await.unwrap();
    assert_eq!(
        request.read_to_end(64).await.unwrap(),
        b"same-address-exact-socket"
    );
    send.write_all(b"PONG").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"PONG");
    println!(
        "M101 exact observations={observations:?} stale_callbacks={} sockets={:?},{:?} stable={stable:?}",
        selector.stale_callbacks.load(SeqCst),
        a.id,
        b.id
    );
    pair.c.close().await;
    pair.v.close().await;
    assert_port_closed(b.local_addr.port());
}
