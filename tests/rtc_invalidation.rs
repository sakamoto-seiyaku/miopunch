//! STEP8 public SDK source invalidation. Real storage and authenticated QUIC;
//! this narrow component test is not browser RTC qualification.
#![cfg(feature = "rtc-probe")]
use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
};
use std::{net::Ipv4Addr, time::Duration};

const MEMBER: &[u8] = b"miopunch/rtc-invalidation-sdk-test/1";

async fn connect(a: &Endpoint, b: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(3), async {
        let address = EndpointAddr::new(b.id()).with_ip_addr(*b.addr().ip_addrs().next().unwrap());
        let (local, remote) = tokio::join!(a.connect(address, alpn), async {
            b.accept().await.unwrap().await
        });
        (local.unwrap(), remote.unwrap())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn durable_current_credential_install_invalidates_old_member_proof_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let admin_path = directory.path().join("admin");
    let member_path = directory.path().join("member");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let mut member = NativeNetwork::join(&member_path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let admin_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let member_endpoint = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (request, served) =
        connect(&member_endpoint, &admin_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (requested, sent) = tokio::join!(
        member.request_manual_admission(&request),
        admin.serve_manual_admission(&served)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    sent.unwrap();
    let pending = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&pending).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            member.request_manual_admission(&request).await.unwrap()
        else {
            panic!("approved material");
        };
        member.complete_manual_admission(material).await.unwrap();
    };
    let (_, sent) = tokio::join!(receive, admin.serve_manual_admission(&served));
    sent.unwrap();
    request.close(0u32.into(), b"admission complete");
    served.close(0u32.into(), b"admission complete");
    for node in [&admin, &member] {
        node.admission_gate()
            .register_protocol(MEMBER, ProtocolScope::Member);
    }
    admin_endpoint.set_alpns(vec![MEMBER.to_vec()]);
    member_endpoint.set_alpns(vec![MEMBER.to_vec()]);
    let (old_member, old_admin) = connect(&member_endpoint, &admin_endpoint, MEMBER).await;
    let gate = admin.admission_gate();
    let old_generation = gate
        .proof_for(&old_admin)
        .unwrap()
        .scope()
        .secret_generation();
    let old_members = admin.status().members;
    let old_id = admin.status().node_id;
    assert!(gate.allows(&old_admin, ProtocolScope::Member));
    let installed = admin.invalidate_current_credential_fixture().await.unwrap();
    assert_eq!(installed, old_generation + 1);
    assert_eq!(
        admin.credential_generation_fixture().await.unwrap(),
        (installed, true)
    );
    assert_eq!(
        admin.status().members,
        old_members,
        "only the credential changes"
    );
    assert!(!gate.allows(&old_admin, ProtocolScope::Member));
    assert!(gate.proof_for(&old_admin).is_none());
    tokio::time::timeout(Duration::from_millis(300), async {
        tokio::join!(old_member.closed(), old_admin.closed());
    })
    .await
    .unwrap();
    tokio::join!(admin_endpoint.close(), member_endpoint.close());
    admin.close().await;
    member.close().await;
    let reopened = NativeNetwork::open(&admin_path).await.unwrap();
    assert_eq!(reopened.status().node_id, old_id);
    assert_eq!(reopened.status().members, old_members);
    assert_eq!(
        reopened.credential_generation_fixture().await.unwrap(),
        (installed, true),
        "real durable current generation"
    );
    reopened.close().await;
}
