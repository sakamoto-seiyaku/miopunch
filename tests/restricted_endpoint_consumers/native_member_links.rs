use iroh::endpoint::Connection;
use miopunch_native_probe::member_transport::{
    MemberDataRelay, MemberDataTransport, MemberForwarder, MemberSignalingTransport,
};
use std::time::Duration;
pub async fn admitted(
    connection: Connection,
    forward: &MemberForwarder,
    signal: &MemberSignalingTransport,
    relay: &MemberDataRelay,
    data: &MemberDataTransport,
) {
    let lease = Duration::from_secs(1);
    let _ = forward.accept_link(&connection, lease).await;
    let _ = signal.attach(&connection, lease).await;
    let _ = relay.accept_link(&connection, lease).await;
    let _ = data.attach(&connection, lease).await;
}
