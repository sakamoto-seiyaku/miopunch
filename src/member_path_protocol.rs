//! Bounded private relay validation on the authenticated original M.
//! Negotiation runs on the already authenticated original QUIC connection.
use std::io;

use iroh::{
    RelayUrl,
    endpoint::{ControlExchange, ControlMessageFormat},
};

/// Relay candidate exchange shared by the native and browser adapters.
/// Its private stream belongs to the already authenticated original M.
pub struct RelayValidation {
    pub candidates: Vec<RelayUrl>,
    pub stream_id: String,
    exchange: ControlExchange,
    stable_id: usize,
    binding: [u8; 32],
}
pub struct RelayContinuity {
    pub stable_unchanged: bool,
    pub binding_unchanged: bool,
}

fn relay_binding(
    connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
) -> io::Result<[u8; 32]> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"miopunch/member-path-continuity/1", b"")
        .map_err(|_| io::Error::other("original member exporter unavailable"))?;
    Ok(binding)
}

impl RelayValidation {
    pub async fn begin(
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        local: Vec<RelayUrl>,
    ) -> io::Result<Self> {
        let stable_id = connection.stable_id();
        let binding = relay_binding(connection)?;
        let exchange = if connection.side().is_client() {
            connection.open_control_exchange().await?
        } else {
            connection.accept_control_exchange().await?
        };
        let encoded = serde_json::to_vec(&local).map_err(io::Error::other)?;
        if local.len() > 4 || encoded.len() > 1024 {
            return Err(io::Error::other("relay candidate message too large"));
        }
        let format = ControlMessageFormat::LengthPrefixed16 { maximum: 1024 };
        exchange.send_message(format, &encoded).await?;
        let candidates = async {
            let encoded = exchange.recv_message(format).await?;
            let candidates: Vec<RelayUrl> =
                serde_json::from_slice(&encoded).map_err(io::Error::other)?;
            if candidates.len() > 4 {
                return Err(io::Error::other("too many peer relay candidates"));
            }
            Ok(candidates)
        }
        .await;
        let candidates = match candidates {
            Ok(candidates) => candidates,
            Err(error) => {
                // The original SendStream drop finished its already written
                // candidate reply even when the peer offer was rejected.
                // Preserve that FIN instead of resetting those valid bytes.
                let _ = exchange.finish_send().await;
                return Err(error);
            }
        };
        Ok(Self {
            candidates,
            stream_id: exchange.id().to_string(),
            exchange,
            stable_id,
            binding,
        })
    }

    /// One-byte bilateral confirmation inside the common coordinator exchange.
    /// Only the boolean continuity result may enter diagnostics.
    pub async fn finish(
        self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        validation: io::Result<()>,
    ) -> io::Result<RelayContinuity> {
        let format = ControlMessageFormat::EndTerminated { maximum: 1 };
        self.exchange
            .send_message(format, &[u8::from(validation.is_ok())])
            .await?;
        let peer = self.exchange.recv_message(format).await?;
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
