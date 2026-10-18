pub fn bypass(control: iroh::endpoint::ControlConnection) -> iroh::endpoint::Connection {
    control.into()
}
