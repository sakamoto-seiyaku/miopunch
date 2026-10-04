//! The existing finite member PING/PONG exchange, shared by native adapters.
use iroh::endpoint::{RecvStream, SendStream};
use std::io;

/// Reply to a complete member request. Admission/current permission belongs to
/// the owning connection handler; this function does not grant a data path.
pub async fn respond(request: &[u8], send: &mut SendStream) -> io::Result<()> {
    if request != b"PING" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected member request",
        ));
    }
    send.write_all(b"PONG").await?;
    send.finish().map_err(io::Error::other)
}

pub async fn serve(mut send: SendStream, mut recv: RecvStream) -> io::Result<()> {
    let request = recv.read_to_end(64).await.map_err(io::Error::other)?;
    respond(&request, &mut send).await
}
