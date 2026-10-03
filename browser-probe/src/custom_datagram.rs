//! Concrete receive metadata shared by the browser signaling carriers.
use iroh::endpoint::transports::RecvInfo;
use iroh_base::CustomAddr;
use std::io::IoSliceMut;

pub(crate) fn copy_received(
    bytes: &[u8],
    source: CustomAddr,
    local: CustomAddr,
    buffer: &mut IoSliceMut<'_>,
    meta: &mut noq_udp::RecvMeta,
    info: &mut RecvInfo,
) -> bool {
    if bytes.len() > buffer.len() {
        return false;
    }
    buffer[..bytes.len()].copy_from_slice(bytes);
    meta.len = bytes.len();
    meta.stride = bytes.len();
    *info = RecvInfo::new(source, Some(local));
    true
}
