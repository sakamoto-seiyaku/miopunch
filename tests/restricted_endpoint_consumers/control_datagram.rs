pub fn bypass(control: &iroh::endpoint::ControlConnection) {
    let _ = control.send_datagram(Vec::new());
}
