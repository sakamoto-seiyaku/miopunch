//! The owned diagnostic gate observes a real lease/source, not a revision mirror.
#![cfg(feature = "rtc-probe")]
#[path = "support/admitted_ip.rs"]
mod admitted_ip;

use iroh::{Endpoint, endpoint::presets};
use miopunch_native_probe::{
    rtc_association::RtcDeadline,
    rtc_invalidation_fixture::ControlCheckpointStage,
    rtc_transport::{RtcTransport, TRANSPORT_ID},
};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
const ALPN: &[u8] = b"miopunch/rtc-owned-control-gate-test/1";
async fn endpoint(transport: &RtcTransport) -> iroh::endpoint::RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(Arc::new(transport.clone()))
        .alpns(vec![ALPN.to_vec()])
        .restricted_application(admitted_ip::IpAdmission(vec![TRANSPORT_ID]))
        .bind()
        .await
        .unwrap()
}
#[tokio::test]
async fn actual_resource_retirement_wakes_the_owned_checkpoint_and_old_release_cannot_revive_it() {
    let a = RtcTransport::new();
    let b = RtcTransport::new();
    let ae = endpoint(&a).await;
    let be = endpoint(&b).await;
    let (left, right) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(ae.connect(be.addr(), ALPN), async {
            be.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let (left, right) = (left.unwrap(), right.unwrap());
    let lease = a.reserve(&left, || true).unwrap();
    let peer = b.reserve(&right, || true).unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    a.arm_control_fixture(ControlCheckpointStage::ChannelReady)
        .unwrap();
    let caller = a.clone();
    let original = left.clone();
    let task = tokio::spawn(async move {
        let result = caller
            .control_fixture_checkpoint(
                ControlCheckpointStage::ChannelReady,
                &original,
                &lease,
                RtcDeadline::from_caller(n0_future::time::Instant::now() + Duration::from_secs(2)),
                serde_json::json!({"attempt":"owned-component-attempt"}),
                Arc::new(|| serde_json::json!({})),
            )
            .await;
        (result, lease)
    });
    tokio::time::timeout(Duration::from_millis(300), async {
        while a.control_fixture_snapshot().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual checkpoint observed");
    let held = a.control_fixture_snapshot().unwrap();
    assert_eq!(held["held"], true);
    a.set_resource_allowed(false);
    let (result, lease) = tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .expect("real resource source wakes paused attempt")
        .unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(!lease.report().active && !lease.report().qualified);
    assert_eq!(lease.report().tx_bytes + lease.report().rx_bytes, 0);
    assert!(
        a.release_control_fixture("owned-component-attempt")
            .is_err()
    );
    a.set_resource_allowed(true);
    assert!(
        lease.activate().is_err(),
        "re-enabled resource cannot reopen old lease"
    );
    assert_eq!(a.active_associations(), 0);
    left.close(0u32.into(), b"fixture ended");
    right.close(0u32.into(), b"fixture ended");
    peer.close();
    a.clear_control_fixture();
    tokio::join!(ae.close(), be.close());
}
