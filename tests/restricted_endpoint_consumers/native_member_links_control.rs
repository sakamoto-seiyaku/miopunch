use iroh::endpoint::ControlConnection;
use miopunch_native_probe::member_transport::{
    MemberDataRelay, MemberDataTransport, MemberForwarder, MemberSignalingTransport,
};
use std::{sync::Arc, time::Duration};
pub async fn forbidden(
    control: Arc<ControlConnection>,
    forward: &MemberForwarder,
    signal: &MemberSignalingTransport,
    relay: &MemberDataRelay,
    data: &MemberDataTransport,
) {
    let lease = Duration::from_secs(1);
    let _ = forward.accept_link(&control, lease).await;
    let _ = signal.attach(&control, lease).await;
    let _ = relay.accept_link(&control, lease).await;
    let _ = data.attach(&control, lease).await;
}
