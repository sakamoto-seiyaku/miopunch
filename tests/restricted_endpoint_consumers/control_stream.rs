pub fn bypass(control: &iroh::endpoint::ControlConnection) {
    let _ = control.open_bi();
}
