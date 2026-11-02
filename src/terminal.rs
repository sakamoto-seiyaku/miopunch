//! Terminal workspaces and bounded attachment records shared by Native/WASM.
use crate::{
    data_stream::DataStream,
    service_policy::{Service, ServiceAccess},
};
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::io;

pub const TERMINAL_ALPN: &[u8] = b"miopunch/terminal/1";
pub const MAX_BYTES: usize = 16 * 1024;
pub const QUEUE_CAPACITY: usize = 16;
pub const MAX_WORKSPACES: usize = 16;

/// Local provider configuration. `directory` is an absolute Linux path, also
/// when the provider runs on Windows (inside the explicitly selected WSL distro).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalProvider {
    pub directory: String,
    #[serde(default)]
    pub distribution: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalRequest {
    Create,
    List,
    End {
        workspace: String,
    },
    Attach {
        workspace: String,
        columns: u16,
        rows: u16,
    },
}
impl TerminalRequest {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::End { workspace } => validate_workspace(workspace),
            Self::Attach {
                workspace,
                columns,
                rows,
            } => {
                validate_workspace(workspace)?;
                validate_size(*columns, *rows)
            }
            Self::Create | Self::List => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalInput {
    Bytes { data: Vec<u8> },
    Resize { columns: u16, rows: u16 },
    Detach,
}
impl TerminalInput {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::Bytes { data } if data.len() > MAX_BYTES => {
                Err(invalid("terminal input too large"))
            }
            Self::Resize { columns, rows } => validate_size(*columns, *rows),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalOutput {
    Bytes { data: Vec<u8> },
    Detached,
}
impl TerminalOutput {
    pub fn validate(&self) -> io::Result<()> {
        if matches!(self, Self::Bytes { data } if data.len() > MAX_BYTES) {
            return Err(invalid("terminal output too large"));
        }
        Ok(())
    }
    pub fn detached(&self) -> bool {
        matches!(self, Self::Detached)
    }
}

pub fn validate_workspace(value: &str) -> io::Result<()> {
    if value.len() != 36
        || !value.starts_with("mio-")
        || !value.as_bytes()[4..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(invalid("invalid terminal workspace"));
    }
    Ok(())
}
pub fn validate_size(columns: u16, rows: u16) -> io::Result<()> {
    if !(2..=512).contains(&columns) || !(2..=512).contains(&rows) {
        return Err(invalid("terminal size outside 2..512"));
    }
    Ok(())
}
pub fn peer_permitted(access: &ServiceAccess, peer: VerifyingKey) -> bool {
    access.member(access.local) && access.member(peer)
        || access
            .guest_pairs_for(Service::Terminal)
            .iter()
            .any(|(id, _)| id == peer.as_bytes())
}
pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(crate) async fn open(
    connection: &iroh::endpoint::Connection,
    request: &TerminalRequest,
) -> io::Result<(DataStream, serde_json::Value)> {
    request.validate()?;
    let mut stream = DataStream::open(connection).await?;
    stream.write_control(request).await?;
    let reply: serde_json::Value = stream.read_control().await?;
    if reply["ok"] != true {
        return Err(invalid("terminal operation rejected"));
    }
    Ok((stream, reply))
}

pub(crate) async fn write_input(
    writer: &mut crate::data_stream::DataWriter,
    message: &TerminalInput,
) -> io::Result<()> {
    message.validate()?;
    let bytes = match message {
        TerminalInput::Bytes { data } => [vec![1], data.clone()].concat(),
        TerminalInput::Resize { columns, rows } => [
            vec![2],
            columns.to_be_bytes().to_vec(),
            rows.to_be_bytes().to_vec(),
        ]
        .concat(),
        TerminalInput::Detach => vec![3],
    };
    writer.write_chunk(&bytes).await
}
pub(crate) async fn read_input(
    reader: &mut crate::data_stream::DataReader,
) -> io::Result<TerminalInput> {
    let bytes = reader.read_chunk().await?;
    let message = match bytes.split_first() {
        Some((1, data)) if data.len() <= MAX_BYTES => TerminalInput::Bytes {
            data: data.to_vec(),
        },
        Some((2, [a, b, c, d])) => TerminalInput::Resize {
            columns: u16::from_be_bytes([*a, *b]),
            rows: u16::from_be_bytes([*c, *d]),
        },
        Some((3, [])) => TerminalInput::Detach,
        _ => return Err(invalid("invalid terminal input")),
    };
    message.validate()?;
    Ok(message)
}
pub(crate) async fn write_output(
    writer: &mut crate::data_stream::DataWriter,
    message: &TerminalOutput,
) -> io::Result<()> {
    message.validate()?;
    let bytes = match message {
        TerminalOutput::Bytes { data } => [vec![1], data.clone()].concat(),
        TerminalOutput::Detached => vec![3],
    };
    writer.write_chunk(&bytes).await
}
pub(crate) async fn read_output(
    reader: &mut crate::data_stream::DataReader,
) -> io::Result<TerminalOutput> {
    let bytes = reader.read_chunk().await?;
    let message = match bytes.split_first() {
        Some((1, data)) if data.len() <= MAX_BYTES => TerminalOutput::Bytes {
            data: data.to_vec(),
        },
        Some((3, [])) => TerminalOutput::Detached,
        _ => return Err(invalid("invalid terminal output")),
    };
    Ok(message)
}

/// The same bounded consumer loop on Native and WASM. Directional futures
/// survive opposite-direction traffic, so a partial record is never discarded.
pub(crate) async fn consume_attachment(
    stream: DataStream,
    connection: &iroh::endpoint::Connection,
    cancel: &tokio_util::sync::CancellationToken,
    mut outgoing: tokio::sync::mpsc::Receiver<TerminalInput>,
    incoming: tokio::sync::mpsc::Sender<TerminalOutput>,
) -> bool {
    use futures_util::FutureExt;
    use std::time::Duration;
    let (mut writer, mut reader) = stream.split();
    let sending = async {
        loop {
            let message = outgoing
                .recv()
                .await
                .ok_or_else(|| invalid("terminal owner ended"))?;
            write_input(&mut writer, &message).await?;
            if matches!(message, TerminalInput::Detach) {
                return Ok::<_, io::Error>(());
            }
        }
    }
    .fuse();
    let receiving = async {
        loop {
            let message = read_output(&mut reader).await?;
            let detached = message.detached();
            incoming
                .send(message)
                .await
                .map_err(|_| invalid("terminal output reader ended"))?;
            if detached {
                return Ok::<_, io::Error>(());
            }
        }
    }
    .fuse();
    let stopped = cancel.cancelled().fuse();
    let closed = connection.closed().fuse();
    futures_util::pin_mut!(sending, receiving, stopped, closed);
    futures_util::select_biased! {
        _ = stopped => false,
        result = receiving => result.is_ok(),
        _ = closed => false,
        result = sending => {
            if result.is_err() { false } else {
                let drain = n0_future::time::timeout(Duration::from_secs(5), receiving).fuse();
                futures_util::pin_mut!(drain);
                futures_util::select_biased! {
                    _ = stopped => false,
                    result = drain => result.is_ok_and(|result| result.is_ok()),
                    _ = closed => false,
                }
            }
        }
    }
}
