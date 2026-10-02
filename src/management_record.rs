//! Complete original signed management record, shared by platform stores.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagementRecord {
    pub(crate) header: Vec<u8>,
    pub(crate) body: Option<Vec<u8>>,
}

impl ManagementRecord {
    pub fn new(header: Vec<u8>, body: Option<&[u8]>) -> Self {
        Self {
            header,
            body: body.map(|bytes| bytes.to_vec()),
        }
    }

    pub fn header(&self) -> &[u8] {
        &self.header
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
}
