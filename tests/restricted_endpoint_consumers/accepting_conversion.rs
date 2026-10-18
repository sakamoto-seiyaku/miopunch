pub fn bypass(accepting: iroh::endpoint::RestrictedAccepting) -> iroh::endpoint::Accepting {
    accepting.into()
}
