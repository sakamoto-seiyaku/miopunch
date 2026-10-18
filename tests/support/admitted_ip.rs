//! Fresh real-IP admission for transport component fixtures, not network membership evidence.
use iroh::endpoint::{AdmissionOutcome, ControlConfirmation, ControlConnection, EndpointAdmission};
use std::io;
#[derive(Debug)]
pub struct IpAdmission(pub Vec<u64>);
impl EndpointAdmission for IpAdmission {
    async fn admit(&self, control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let paths = control.paths();
        let path = paths
            .iter()
            .filter(|path| path.is_ip())
            .max_by_key(|path| path.is_selected())
            .ok_or_else(|| io::Error::other("fixture IP path unavailable"))?
            .id();
        let permit = control.qualify_data_path(path, self.0.clone()).await?;
        Ok(AdmissionOutcome::AcceptWithConfirmation {
            permit,
            confirmation: ControlConfirmation::new(async { Ok(()) }),
        })
    }
    fn is_current(&self, control: &ControlConnection) -> bool {
        control.close_reason().is_none()
    }
}
