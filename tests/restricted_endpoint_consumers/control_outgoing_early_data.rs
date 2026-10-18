pub fn bypass(connecting: iroh::endpoint::ControlConnecting) {
    let _ = connecting.into_0rtt();
}
