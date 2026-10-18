//! Real protected-hook/QUIC/lease boundary. The owned queue bridge is not
//! a browser RTC association and does not qualify the WSS carrier.
use futures_util::future::poll_fn;
use iroh::{
    Endpoint, TransportAddr,
    endpoint::{
        AdmissionOutcome, ControlConfirmation, ControlConnection, ControlMessageFormat,
        EndpointAdmission, RestrictedEndpoint, presets,
    },
};
use iroh_base::CustomAddr;
use miopunch_native_probe::rtc_transport::{
    RtcDatagramIo, RtcRouteLease, RtcTransport, TRANSPORT_ID,
};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

const ALPN: &[u8] = b"miopunch/rtc-control-release-test/1";
#[derive(Default)]
struct Bridge {
    halves: Mutex<[Option<RtcDatagramIo>; 2]>,
    notify: tokio::sync::Notify,
    pumps: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    confirmed: AtomicUsize,
}
#[derive(Clone)]
struct Gate {
    side: usize,
    transport: RtcTransport,
    bridge: Arc<Bridge>,
    owner: Arc<Mutex<Option<RtcRouteLease>>>,
    original: Arc<Mutex<Option<usize>>>,
    current: Arc<AtomicBool>,
    invalidate_before_release: bool,
}
impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate").field("side", &self.side).finish()
    }
}
impl EndpointAdmission for Gate {
    async fn admit(&self, original: &ControlConnection) -> io::Result<AdmissionOutcome> {
        *self.original.lock().unwrap() = Some(original.stable_id());
        let exchange = if original.side().is_client() {
            original.open_control_exchange().await?
        } else {
            original.accept_control_exchange().await?
        };
        let current = self.current.clone();
        let lease = self
            .transport
            .reserve(original, move || current.load(Ordering::Acquire))?;
        exchange
            .send_message(
                ControlMessageFormat::Fixed { length: 16 },
                lease.local_addr().data(),
            )
            .await?;
        let mut address = [0; 16];
        address.copy_from_slice(
            &exchange
                .recv_message(ControlMessageFormat::Fixed { length: 16 })
                .await?,
        );
        let remote = CustomAddr::from_parts(TRANSPORT_ID, &address);
        lease.set_remote(remote.clone())?;
        lease.activate()?;
        self.bridge.halves.lock().unwrap()[self.side] = Some(lease.io());
        self.bridge.notify.notify_waiters();
        let destination = loop {
            let notified = self.bridge.notify.notified();
            let peer = self.bridge.halves.lock().unwrap()[1 - self.side].clone();
            if let Some(peer) = peer {
                break peer;
            }
            notified.await;
        };
        let source = lease.io();
        self.bridge
            .pumps
            .lock()
            .unwrap()
            .push(tokio::spawn(async move {
                while let Ok(packet) = poll_fn(|cx| source.poll_outbound(cx)).await {
                    if destination.receive(packet).is_err() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }));
        if original.side().is_client() {
            original.open_control_path(remote.clone()).await?;
        }
        let probe = loop {
            match lease.start_probe(original) {
                Ok(probe) => break probe,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => return Err(e),
            }
        };
        for path in original
            .paths()
            .iter()
            .filter(|path| path.remote_addr().is_custom())
        {
            assert_eq!(
                path.stats().frame_tx.stream,
                0,
                "pending path cannot carry private STREAM"
            );
            assert_eq!(path.stats().frame_tx.datagram, 0);
        }
        while !probe.is_acknowledged()? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        lease.retain(original, &probe)?;
        exchange
            .send_message(ControlMessageFormat::Fixed { length: 3 }, b"ack")
            .await?;
        let mut ack = [0; 3];
        ack.copy_from_slice(
            &exchange
                .recv_message(ControlMessageFormat::Fixed { length: 3 })
                .await?,
        );
        assert_eq!(&ack, b"ack");
        original
            .retain_control_data_transport_path(remote.clone())
            .await?;
        assert!(
            original.paths().iter().any(|path| path.remote_addr()
                == &TransportAddr::Custom(remote.clone())
                && path.is_selected()),
            "restricted connection pins exact qualified RTC"
        );
        // Isolate old IP loss for the continuation observation, only in this
        // fixture. Final authorization must replace this mask while retaining
        // the qualified exact route and excluding type-wide custom grants.
        original
            .test_limit_transport_paths(vec![TransportAddr::Custom(remote.clone())])
            .await?;
        for path in original.paths().iter().filter(|path| path.is_ip()) {
            let _ = path.close();
        }
        exchange
            .send_message(
                ControlMessageFormat::Fixed { length: 19 },
                b"same private stream",
            )
            .await?;
        let mut confirm = [0; 19];
        confirm.copy_from_slice(
            &exchange
                .recv_message(ControlMessageFormat::Fixed { length: 19 })
                .await?,
        );
        assert_eq!(&confirm, b"same private stream");
        exchange.finish_send().await?;

        *self.owner.lock().unwrap() = Some(lease);
        if self.invalidate_before_release {
            self.bridge.confirmed.fetch_add(1, Ordering::AcqRel);
            self.bridge.notify.notify_waiters();
            loop {
                let notified = self.bridge.notify.notified();
                if self.bridge.confirmed.load(Ordering::Acquire) == 2 {
                    break;
                }
                notified.await;
            }
            // Fault after both private reads completed, so this story tests
            // queued admission rather than interruption of confirmation.
            self.current.store(false, Ordering::Release);
        }
        let path = original
            .paths()
            .iter()
            .find(|path| path.remote_addr() == &TransportAddr::Custom(remote.clone()))
            .unwrap()
            .id();

        let permit = original.qualify_data_path(path, vec![]).await?;

        Ok(AdmissionOutcome::AcceptWithConfirmation {
            permit,
            confirmation: ControlConfirmation::new(async { Ok(()) }),
        })
    }
    fn is_current(&self, original: &ControlConnection) -> bool {
        self.current.load(Ordering::Acquire) && original.close_reason().is_none()
    }
}
async fn endpoint(gate: Gate) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(Arc::new(gate.transport.clone()))
        .alpns(vec![ALPN.to_vec()])
        .restricted_application(gate)
        .bind()
        .await
        .unwrap()
}
#[tokio::test]
async fn protected_hooks_confirm_on_exact_rtc_before_original_m_is_released() {
    tokio::time::timeout(Duration::from_secs(8), async {
        let bridge = Arc::new(Bridge::default());
        let gate = |side| Gate {
            side,
            transport: RtcTransport::new(),
            bridge: bridge.clone(),
            owner: Arc::new(Mutex::new(None)),
            original: Arc::new(Mutex::new(None)),
            current: Arc::new(AtomicBool::new(true)),
            invalidate_before_release: false,
        };
        let left = gate(0);
        let right = gate(1);
        let a = endpoint(left.clone()).await;
        let b = endpoint(right.clone()).await;
        let (local, remote) = tokio::join!(a.connect(b.addr(), ALPN), async {
            b.accept().await.unwrap().await
        });

        let (local, remote) = (local.unwrap(), remote.unwrap());
        assert_eq!(Some(local.stable_id()), *left.original.lock().unwrap());
        assert_eq!(Some(remote.stable_id()), *right.original.lock().unwrap());
        let mut l = [0; 32];
        let mut r = [0; 32];
        local
            .export_keying_material(&mut l, b"control-release-test", &[])
            .unwrap();
        remote
            .export_keying_material(&mut r, b"control-release-test", &[])
            .unwrap();
        assert!(l == r, "original authenticated M TLS binding comparison");
        let (mut request, mut reply) = local.open_bi().await.unwrap();
        request.write_all(b"released original M").await.unwrap();
        request.finish().unwrap();
        let (mut response, mut incoming) = remote.accept_bi().await.unwrap();
        assert_eq!(
            incoming.read_to_end(32).await.unwrap(),
            b"released original M"
        );
        response
            .write_all(b"normal data over exact RTC")
            .await
            .unwrap();
        response.finish().unwrap();
        assert_eq!(
            reply.read_to_end(64).await.unwrap(),
            b"normal data over exact RTC"
        );
        for connection in [&local, &remote] {
            assert!(
                connection
                    .paths()
                    .iter()
                    .any(|p| p.remote_addr().is_custom()
                        && p.is_selected()
                        && p.stats().frame_tx.stream > 0)
            );
            connection.close(0u32.into(), b"owned fixture ended");
        }
        left.owner.lock().unwrap().take();
        right.owner.lock().unwrap().take();
        for pump in bridge.pumps.lock().unwrap().drain(..) {
            pump.abort();
        }
        tokio::join!(a.close(), b.close());
    })
    .await
    .expect("owned protected release completes within fixture limit");
}

#[tokio::test]
async fn queued_final_release_rechecks_the_exact_route_after_hook_confirmation() {
    let bridge = Arc::new(Bridge::default());
    let gate = |side| Gate {
        side,
        transport: RtcTransport::new(),
        bridge: bridge.clone(),
        owner: Arc::new(Mutex::new(None)),
        original: Arc::new(Mutex::new(None)),
        current: Arc::new(AtomicBool::new(true)),
        invalidate_before_release: true,
    };
    let left = gate(0);
    let right = gate(1);
    let a = endpoint(left.clone()).await;
    let b = endpoint(right.clone()).await;
    let (local, remote) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(a.connect(b.addr(), ALPN), async {
            b.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(
        local.is_err() && remote.is_err(),
        "qualification changed after confirmation; neither normal handle may be released"
    );
    left.owner.lock().unwrap().take();
    right.owner.lock().unwrap().take();
    for pump in bridge.pumps.lock().unwrap().drain(..) {
        pump.abort();
    }
    tokio::join!(a.close(), b.close());
}

#[derive(Debug, Clone)]
struct RejectSignalingSeal(Arc<AtomicBool>, Arc<tokio::sync::Barrier>);
impl EndpointAdmission for RejectSignalingSeal {
    async fn admit(&self, original: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let signaling = original
            .paths()
            .iter()
            .find_map(|path| match path.remote_addr() {
                TransportAddr::Custom(address) => Some(address.clone()),
                _ => None,
            })
            .expect("this authenticated M actually started on MQTT");
        assert_eq!(
            signaling.id(),
            miopunch_native_probe::mqtt_transport::TRANSPORT_ID
        );
        let error = original
            .retain_control_data_transport_path(signaling)
            .await
            .expect_err("legacy signaling carrier cannot become an exact data grant");
        assert!(error.to_string().contains("connection-owned qualification"));
        self.0.store(true, Ordering::Release);
        self.1.wait().await;
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "signaling has no normal data grant",
        ))
    }
    fn is_current(&self, original: &ControlConnection) -> bool {
        original.close_reason().is_none()
    }
}
#[tokio::test]
async fn actual_mqtt_control_route_cannot_be_sealed_as_an_ordinary_data_path() {
    use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse::<SocketAddr>().unwrap();
    let observed = [
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    ];
    let carriers = [
        Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default()))),
        Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default()))),
    ];
    let checked = Arc::new(tokio::sync::Barrier::new(2));
    let mut endpoints = Vec::new();
    for index in 0..2 {
        endpoints.push(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_ip_transports()
                .clear_relay_transports()
                .add_custom_transport(carriers[index].clone())
                .alpns(vec![ALPN.to_vec()])
                .restricted_application(RejectSignalingSeal(
                    observed[index].clone(),
                    checked.clone(),
                ))
                .bind()
                .await
                .unwrap(),
        );
        carriers[index].wait_ready().await.unwrap();
    }
    let target = iroh::EndpointAddr::new(endpoints[1].id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&carriers[1].route().unwrap()).unwrap(),
    )]);
    let (local, remote) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(endpoints[0].connect(target, ALPN), async {
            endpoints[1].accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(
        local.is_err() && remote.is_err(),
        "neither signaling M yields a normal data handle"
    );
    assert!(
        observed.iter().all(|value| value.load(Ordering::Acquire)),
        "both restricted hooks rejected the actual signaling route"
    );
    assert!(
        carriers.iter().all(|carrier| carrier.diagnostics().1),
        "facility remains ready while ordinary signaling use is refused"
    );
    tokio::join!(endpoints[0].close(), endpoints[1].close());
}
