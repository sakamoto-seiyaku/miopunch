//! Bounded private relay validation on the authenticated original M.
//! Negotiation runs on the already authenticated original QUIC connection.
use std::io;

use iroh::{
    RelayUrl,
    endpoint::{Connection, RecvStream, SendStream},
};

/// Relay candidate exchange shared by the native and browser adapters.
/// Its private stream belongs to the already authenticated original M.
pub struct RelayValidation {
    pub candidates: Vec<RelayUrl>,
    pub stream_id: String,
    send: SendStream,
    recv: RecvStream,
    stable_id: usize,
    binding: [u8; 32],
}
pub struct RelayContinuity {
    pub stable_unchanged: bool,
    pub binding_unchanged: bool,
}

fn relay_binding(connection: &Connection) -> io::Result<[u8; 32]> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"miopunch/member-path-continuity/1", b"")
        .map_err(|_| io::Error::other("original member exporter unavailable"))?;
    Ok(binding)
}

impl RelayValidation {
    pub async fn begin(connection: &Connection, local: Vec<RelayUrl>) -> io::Result<Self> {
        let stable_id = connection.stable_id();
        let binding = relay_binding(connection)?;
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
        };
        let encoded = serde_json::to_vec(&local).map_err(io::Error::other)?;
        if local.len() > 4 || encoded.len() > 1024 {
            return Err(io::Error::other("relay candidate message too large"));
        }
        send.write_all(&(encoded.len() as u16).to_be_bytes())
            .await?;
        send.write_all(&encoded).await?;
        let mut length = [0; 2];
        recv.read_exact(&mut length)
            .await
            .map_err(io::Error::other)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length > 1024 {
            return Err(io::Error::other("peer relay candidate message too large"));
        }
        let mut encoded = vec![0; length];
        recv.read_exact(&mut encoded)
            .await
            .map_err(io::Error::other)?;
        let candidates: Vec<RelayUrl> =
            serde_json::from_slice(&encoded).map_err(io::Error::other)?;
        if candidates.len() > 4 {
            return Err(io::Error::other("too many peer relay candidates"));
        }
        Ok(Self {
            candidates,
            stream_id: send.id().to_string(),
            send,
            recv,
            stable_id,
            binding,
        })
    }

    /// One-byte bilateral confirmation inside the common coordinator exchange.
    /// Only the boolean continuity result may enter diagnostics.
    pub async fn finish(
        mut self,
        connection: &Connection,
        validation: io::Result<()>,
    ) -> io::Result<RelayContinuity> {
        self.send.write_all(&[u8::from(validation.is_ok())]).await?;
        self.send.finish()?;
        let peer = self.recv.read_to_end(1).await.map_err(io::Error::other)?;
        validation?;
        if peer != [1] {
            return Err(io::Error::other("relay validation failed at peer"));
        }
        let stable_unchanged = connection.stable_id() == self.stable_id;
        let binding_unchanged = relay_binding(connection)? == self.binding;
        if !stable_unchanged || !binding_unchanged {
            return Err(io::Error::other("relay original member continuity changed"));
        }
        Ok(RelayContinuity {
            stable_unchanged,
            binding_unchanged,
        })
    }
}
