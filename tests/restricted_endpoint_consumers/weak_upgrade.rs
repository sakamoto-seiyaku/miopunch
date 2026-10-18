pub fn bypass(control: &iroh::endpoint::ControlConnection) {
    let _ = control.weak_handle().upgrade();
}
