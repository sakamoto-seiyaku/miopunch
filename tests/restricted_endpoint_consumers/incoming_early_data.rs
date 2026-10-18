pub fn bypass(accepting: iroh::endpoint::RestrictedAccepting) {
    let _ = accepting.into_0rtt();
}
