pub fn bypass(incoming: iroh::endpoint::RestrictedIncoming) -> iroh::endpoint::Incoming {
    incoming.into()
}
