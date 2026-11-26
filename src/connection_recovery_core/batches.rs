//! Candidate updates and the original-path commit share one framed exchange.
use super::*;

pub(crate) const FORMAT: ControlMessageFormat = ControlMessageFormat::LengthPrefixed16 {
    maximum: MAX_MESSAGE,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Offer {
    #[serde(flatten)]
    pub offer: EndpointAddr,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub batched: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum Frame {
    Candidates(EndpointAddr),
    Ready(RepairConfirmation),
    Committed,
}

/// The count bounds one validation submission. The encoded frame also bounds
/// larger custom addresses, independently of the complete advertised set.
pub(crate) fn offers(offer: &EndpointAddr) -> io::Result<Vec<EndpointAddr>> {
    let mut batches = Vec::new();
    let mut batch = EndpointAddr::new(offer.id);
    for address in &offer.addrs {
        let mut next = batch.clone();
        next.addrs.insert(address.clone());
        if next.addrs.len() > MAX_CANDIDATES
            || encode_message(&Frame::Candidates(next.clone())).is_err()
        {
            if batch.addrs.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "repair address exceeds message bound",
                ));
            }
            batches.push(batch);
            next = EndpointAddr::new(offer.id).with_addrs([address.clone()]);
            encode_message(&Frame::Candidates(next.clone()))?;
        }
        batch = next;
    }
    batches.push(batch);
    Ok(batches)
}

async fn write(exchange: &ControlExchange, frame: &Frame) -> io::Result<()> {
    exchange.send_message(FORMAT, &encode_message(frame)?).await
}

async fn read(exchange: &ControlExchange) -> io::Result<Frame> {
    serde_json::from_slice(&exchange.recv_message(FORMAT).await?).map_err(io::Error::other)
}

pub(crate) async fn exchange<E: RecoveryEndpoint + ?Sized, C: AuthenticatedControl + ?Sized>(
    endpoint: &E,
    original: &Connection,
    control: &C,
    exchange: &ControlExchange,
    offer: EndpointAddr,
    first: EndpointAddr,
    permits: &impl Fn(&TransportAddr) -> bool,
    check: &impl Fn(&Connection, &C) -> io::Result<()>,
    check_original: &impl Fn(&Connection) -> io::Result<()>,
) -> io::Result<ConnectionPathRecovery> {
    if first.id != original.remote_id() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let (updates, receiver) = tokio::sync::watch::channel(first);
    let send = async {
        let send_candidates = async {
            for batch in offers(&offer)? {
                check(original, control)?;
                write(exchange, &Frame::Candidates(batch)).await?;
            }
            Ok::<_, io::Error>(())
        };
        let ((), path) = futures_util::try_join!(
            send_candidates,
            validate_updates(endpoint, original, control, receiver, permits, check)
        )?;
        write(
            exchange,
            &Frame::Ready(RepairConfirmation {
                path: path.address.clone(),
                fresh_ack: true,
                probe_watermark: path.watermark,
            }),
        )
        .await?;
        Ok::<_, io::Error>(path)
    };
    let receive = async {
        loop {
            check(original, control)?;
            match read(exchange).await? {
                Frame::Candidates(batch) => {
                    if batch.id != original.remote_id() || batch.addrs.len() > MAX_CANDIDATES {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    updates.send_modify(|current| current.addrs.extend(batch.addrs));
                }
                Frame::Ready(confirmation) => {
                    if !confirmation.fresh_ack
                        || !offer.addrs.contains(&confirmation.path)
                        || !permits(&confirmation.path)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "peer did not validate an offered path",
                        ));
                    }
                    return Ok(());
                }
                Frame::Committed => return Err(io::ErrorKind::InvalidData.into()),
            }
        }
    };
    let (mut path, ()) = futures_util::try_join!(send, receive)?;
    check(original, control)?;
    write(exchange, &Frame::Committed).await?;
    if !matches!(read(exchange).await?, Frame::Committed) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // Peer COMMITTED ends the private contact's protocol role. Its owner can
    // now close C, while the original's authorization and path proof remain live.
    check_original(original)?;
    if !permits(&path.address) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    path.retain_candidate()?;
    let _ = exchange.finish_send_confirmed().await;
    Ok(ConnectionPathRecovery {
        stable_id: original.stable_id(),
        path: path.address,
        path_id: path.id,
        probe_watermark: path.watermark,
        control_stable_id: control.stable_id(),
        peer_confirmed: true,
    })
}
