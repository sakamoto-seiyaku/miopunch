pub fn bypass(connecting: iroh::endpoint::RestrictedConnecting) {
    let _ = connecting.into_0rtt();
}
