pub fn bypass(accepting: iroh::endpoint::ControlAccepting) {
    let _ = accepting.into_0rtt();
}
