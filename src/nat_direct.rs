//! Protected, bounded direct UDP candidates before any punching recommendation.
use crate::{
    admission_gate::AdmissionGate, control_session::ControlSession, nat_selector::LearningScope,
};
use iroh::endpoint::{ControlExchange, ControlMessageFormat, ObservationValidity};
use serde::{Deserialize, Serialize};
use std::{io, net::SocketAddr, time::Duration};

const LIMIT: usize = 16 * 1024;
const WAIT: Duration = Duration::from_secs(3);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Frame {
    Hello {
        network: [u8; 32],
        generation: u64,
    },
    Offer {
        nonce: [u8; 16],
        addresses: Vec<SocketAddr>,
    },
    Ready {
        nonce: [u8; 16],
    },
    Result {
        nonce: [u8; 16],
        available: bool,
    },
}

async fn send(exchange: &ControlExchange, frame: Frame) -> io::Result<()> {
    let bytes = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    exchange
        .send_message(
            ControlMessageFormat::EndTerminated { maximum: LIMIT },
            &bytes,
        )
        .await
}
async fn receive(exchange: &ControlExchange) -> io::Result<Frame> {
    serde_json::from_slice(
        &exchange
            .recv_message(ControlMessageFormat::EndTerminated { maximum: LIMIT })
            .await?,
    )
    .map_err(io::Error::other)
}

fn validate(addresses: &[SocketAddr]) -> io::Result<()> {
    if addresses.len() > 64
        || addresses
            .iter()
            .any(|a| a.port() == 0 || a.ip().is_unspecified() || a.ip().is_multicast())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid direct candidates",
        ));
    }
    Ok(())
}

/// Captured validity remains local. A peer cannot replace its network epoch.
#[derive(Clone, Debug)]
pub(crate) struct DirectPath {
    pub remote: SocketAddr,
    pub scope: LearningScope,
    pub generation: ObservationValidity,
    pub connection_id: usize,
}

impl DirectPath {
    pub fn check(&self, control: &ControlSession, gate: &AdmissionGate) -> io::Result<()> {
        if control.stable_id() != self.connection_id
            || !self.generation.is_current()
            || control.nat_learning_scope(gate).as_ref() != Some(&self.scope)
            || control.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "direct path scope changed",
            ));
        }
        Ok(())
    }
}

fn ip_path(control: &ControlSession) -> Option<SocketAddr> {
    control
        .connection_paths()
        .into_iter()
        .find_map(|a| match a {
            iroh_base::TransportAddr::Ip(a) => Some(a),
            _ => None,
        })
}

pub(crate) async fn attempt(
    control: &ControlSession,
    gate: &AdmissionGate,
) -> io::Result<Option<DirectPath>> {
    let scope = control.nat_learning_scope(gate).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "direct member proof required",
        )
    })?;
    let generation = control.endpoint().udp_network_generation();
    let connection_id = control.stable_id();
    let check = || {
        if !generation.is_current()
            || control.nat_learning_scope(gate).as_ref() != Some(&scope)
            || control.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "direct path scope changed",
            ));
        }
        Ok(())
    };
    let mut addresses = control
        .endpoint()
        .addr()
        .ip_addrs()
        .copied()
        .collect::<Vec<_>>();
    addresses.retain(|a| !a.ip().is_unspecified() && a.port() > 0);
    // LAN first, then IPv6, then observed/public IPv4. Stable order within groups.
    addresses.sort_by_key(|a| match a.ip() {
        std::net::IpAddr::V4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => 0,
        std::net::IpAddr::V6(_) => 1,
        _ => 2,
    });
    addresses.truncate(64);
    let operation = async {
        check()?;
        let (nonce, remote) = if control.is_dialing_side() {
            let exchange = control.open_control_exchange().await?;
            send(
                &exchange,
                Frame::Hello {
                    network: scope.network,
                    generation: scope.secret_generation,
                },
            )
            .await?;
            let Frame::Offer {
                nonce,
                addresses: remote,
            } = receive(&exchange).await?
            else {
                return Err(io::Error::other("direct offer required"));
            };
            validate(&remote)?;
            check()?;
            let exchange = control.open_control_exchange().await?;
            send(&exchange, Frame::Offer { nonce, addresses }).await?;
            if !matches!(receive(&exchange).await?, Frame::Ready { nonce: ack } if ack == nonce) {
                return Err(io::Error::other("direct nonce mismatch"));
            }
            (nonce, remote)
        } else {
            let exchange = control.accept_control_exchange().await?;
            if !matches!(receive(&exchange).await?, Frame::Hello { network, generation }
                if network == scope.network && generation == scope.secret_generation)
            {
                return Err(io::Error::other("direct scope mismatch"));
            }
            let nonce = rand::random();
            check()?;
            send(&exchange, Frame::Offer { nonce, addresses }).await?;
            let exchange = control.accept_control_exchange().await?;
            let Frame::Offer {
                nonce: offered,
                addresses: remote,
            } = receive(&exchange).await?
            else {
                return Err(io::Error::other("direct peer offer required"));
            };
            if offered != nonce {
                return Err(io::Error::other("old direct round"));
            }
            validate(&remote)?;
            check()?;
            send(&exchange, Frame::Ready { nonce }).await?;
            (nonce, remote)
        };
        if control.is_dialing_side() {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            for address in &remote {
                check()?;
                if ip_path(control).is_some() {
                    break;
                }
                println!(
                    "NAT_DIRECT_CANDIDATE stable_id={} target={address}",
                    control.stable_id()
                );
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let _ = tokio::time::timeout(
                    remaining.min(Duration::from_millis(250)),
                    control.open_data_path_report(*address),
                )
                .await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            while ip_path(control).is_none()
                && tokio::time::Instant::now() < deadline
                && !remote.is_empty()
            {
                check()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            check()?;
            let exchange = control.open_control_exchange().await?;
            send(
                &exchange,
                Frame::Result {
                    nonce,
                    available: ip_path(control).is_some(),
                },
            )
            .await?;
            if !matches!(receive(&exchange).await?, Frame::Result { nonce: result, available: true } if result == nonce)
            {
                return Ok(None);
            }
        } else {
            let exchange = control.accept_control_exchange().await?;
            let Frame::Result {
                nonce: result,
                available,
            } = receive(&exchange).await?
            else {
                return Err(io::Error::other("direct result required"));
            };
            if result != nonce {
                return Err(io::Error::other("old direct result"));
            }
            check()?;
            let available = available && ip_path(control).is_some();
            send(&exchange, Frame::Result { nonce, available }).await?;
            if !available {
                return Ok(None);
            }
        }
        check()?;
        Ok(ip_path(control).map(|remote| DirectPath {
            remote,
            scope: scope.clone(),
            generation: generation.clone(),
            connection_id,
        }))
    };
    tokio::time::timeout(WAIT + Duration::from_secs(2), operation)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "direct exchange interrupted"))?
}
