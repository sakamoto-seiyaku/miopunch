//! Bounded WebSocket ownership for the Web service; no general socket tunnel.
use super::{
    ClientError, Connection, DataStream, EndpointAddr, FlowOwner, MAX_FLOWS, NativeClient,
    NativeWeb, RequestHead, ResponseHead, forward_header, web,
};
use crate::web::WebSocketMessage;
use futures_util::{SinkExt, StreamExt};
use p2panda_core::VerifyingKey;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub(super) struct SocketSlot {
    outgoing: mpsc::Sender<WebSocketMessage>,
    incoming: Mutex<mpsc::Receiver<WebSocketMessage>>,
    cancel: CancellationToken,
    outcome: Arc<std::sync::Mutex<Option<bool>>>,
}

fn http_origin(url: &str) -> Result<String, ClientError> {
    let mut url = url::Url::parse(url)?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Err("WebSocket requires ws or wss".into()),
    };
    url.set_scheme(scheme)
        .map_err(|_| "invalid WebSocket URL")?;
    super::origin(url.as_str())
}
fn target_message(message: WebSocketMessage) -> Result<tokio_websockets::Message, ClientError> {
    message.validate()?;
    Ok(match message {
        WebSocketMessage::Text { text } => tokio_websockets::Message::text(text),
        WebSocketMessage::Binary { data } => tokio_websockets::Message::binary(data),
        WebSocketMessage::Close { code, reason } => {
            let code = tokio_websockets::CloseCode::try_from(code)?;
            if code.is_reserved() {
                return Err("reserved WebSocket close code".into());
            }
            tokio_websockets::Message::close(Some(code), &reason)
        }
    })
}

impl NativeWeb {
    pub(super) async fn serve_socket(
        &self,
        owner: &FlowOwner,
        connection: Connection,
        mut stream: DataStream,
        request: RequestHead,
    ) -> Result<(), ClientError> {
        self.phase(owner, "checking_socket_target");
        if request.method != "GET" {
            return Err("WebSocket requires GET".into());
        }
        self.authorized_client(owner, &http_origin(&request.url)?)?;
        let tls = self
            .state
            .lock()
            .unwrap()
            .tls
            .clone()
            .ok_or("Web service off")?;
        let connector = tokio_websockets::Connector::Rustls(tokio_rustls::TlsConnector::from(tls));
        let mut builder = tokio_websockets::ClientBuilder::new()
            .uri(&request.url)?
            .connector(&connector)
            .limits(
                tokio_websockets::Limits::default().max_payload_len(Some(web::MAX_SOCKET_MESSAGE)),
            );
        for (name, value) in request.headers {
            if forward_header(&name)
                && !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "sec-websocket-key" | "sec-websocket-version"
                )
            {
                builder = builder.add_header(name.parse()?, value.parse()?)?;
            }
        }
        self.phase(owner, "connecting_socket_target");
        let (socket, response) =
            tokio::time::timeout(Duration::from_secs(15), builder.connect()).await??;
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| name.as_str() == "sec-websocket-protocol")
            .map(|(name, value)| Ok((name.to_string(), value.to_str()?.to_string())))
            .collect::<Result<Vec<_>, reqwest::header::ToStrError>>()?;
        web::write_head(
            &mut stream,
            &ResponseHead {
                url: request.url,
                status: 101,
                headers,
            },
        )
        .await?;
        self.phase(owner, "socket_duplex");
        let (mut output, mut input) = stream.split();
        let (mut target_send, mut target_receive) = socket.split();
        // Keep each directional future alive across opposite-direction work.
        // DataReader::read_chunk is a multi-read operation; selecting a NEW
        // read on every message could discard a partially received record.
        let to_target = async {
            loop {
                let message = web::read_socket_message(&mut input).await?;
                let closed = message.is_close();
                target_send.send(target_message(message)?).await?;
                if closed {
                    return Ok::<_, ClientError>(());
                }
            }
        };
        let from_target = async {
            loop {
                let Some(message) = target_receive.next().await else {
                    return Err("WebSocket target ended without close".into());
                };
                let message = message?;
                let outgoing = if let Some(text) = message.as_text() {
                    Some(WebSocketMessage::Text {
                        text: text.to_owned(),
                    })
                } else if message.is_binary() {
                    Some(WebSocketMessage::Binary {
                        data: message.as_payload().to_vec(),
                    })
                } else if let Some((code, reason)) = message.as_close() {
                    Some(WebSocketMessage::Close {
                        code: code.into(),
                        reason: reason.into(),
                    })
                } else {
                    None
                };
                if let Some(message) = outgoing {
                    let closed = message.is_close();
                    web::write_socket_message(&mut output, &message).await?;
                    if closed {
                        output.finish()?;
                        return Ok::<_, ClientError>(());
                    }
                }
            }
        };
        tokio::pin!(to_target, from_target);
        tokio::select! {
            result = &mut from_target => result?,
            result = &mut to_target => { result?; tokio::time::timeout(Duration::from_secs(5), &mut from_target).await??; },
        }
        self.phase(owner, "awaiting_socket_receipt");
        let reason = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await?;
        if !matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(reason)
            if reason.error_code == 0u32.into() && reason.reason.as_ref() == b"Web operation completed")
        {
            return Err("WebSocket consumer did not complete receipt".into());
        }
        Ok(())
    }
    async fn socket_consumer(
        &self,
        owner: &FlowOwner,
        stream: DataStream,
        mut outgoing: mpsc::Receiver<WebSocketMessage>,
        incoming: mpsc::Sender<WebSocketMessage>,
    ) -> Result<(), ClientError> {
        let (mut writer, mut reader) = stream.split();
        let sending = async {
            loop {
                let message = outgoing.recv().await.ok_or("WebSocket owner ended")?;
                let closed = message.is_close();
                web::write_socket_message(&mut writer, &message).await?;
                if closed {
                    return Ok::<_, ClientError>(());
                }
            }
        };
        let receiving = async {
            loop {
                let message = web::read_socket_message(&mut reader).await?;
                let closed = message.is_close();
                incoming
                    .send(message)
                    .await
                    .map_err(|_| "WebSocket reader ended")?;
                if closed {
                    self.completed(owner);
                    return Ok::<_, ClientError>(());
                }
            }
        };
        tokio::pin!(sending, receiving);
        tokio::select! {
            result = &mut receiving => result,
            result = &mut sending => { result?; tokio::time::timeout(Duration::from_secs(5), &mut receiving).await? },
        }
    }
    fn socket(&self, id: &str) -> Result<Arc<SocketSlot>, ClientError> {
        self.state
            .lock()
            .unwrap()
            .sockets
            .get(&id.parse::<u64>()?)
            .cloned()
            .ok_or_else(|| "unknown WebSocket handle".into())
    }
    async fn socket_joined(&self, id: u64) {
        loop {
            let joined = self.joined.notified();
            if !self.state.lock().unwrap().flows.contains_key(&id) {
                return;
            }
            joined.await;
        }
    }
}

impl NativeClient {
    pub async fn open_web_socket(
        &self,
        address: EndpointAddr,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        http_origin(&url)?;
        let peer = VerifyingKey::from_bytes(address.id.as_bytes())?;
        let access = {
            let node = self.node.lock().await;
            self.services.lock().await.access(&node).await?
        };
        self.web.enforce_access(&access);
        if self.web.state.lock().unwrap().sockets.len() >= MAX_FLOWS {
            return Err("WebSocket handle limit reached".into());
        }
        let owner = self.web.claim(peer, false)?;
        let (connection, stream, ready) = tokio::select! {
            biased;
            _ = owner.cancel.cancelled() => return Err("WebSocket opening cancelled".into()),
            result = async {
                let connection = {
                    let _contact = self.web.connecting.lock().await;
                    self.resources.connect(address, web::WEB_ALPN).await?
                };
                self.web.attach(&owner, connection.clone())?;
                let (stream, ready) = web::open_socket(&connection, url, headers).await?;
                Ok::<_, ClientError>((connection, stream, ready))
            } => result?,
        };
        let (sender, outgoing) = mpsc::channel(16);
        let (incoming, receiver) = mpsc::channel(16);
        let outcome = Arc::new(std::sync::Mutex::new(None));
        let id = owner.id;
        let slot = Arc::new(SocketSlot {
            outgoing: sender,
            incoming: Mutex::new(receiver),
            cancel: owner.cancel.clone(),
            outcome: outcome.clone(),
        });
        let mut tasks = self.web.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        {
            let mut state = self.web.state.lock().unwrap();
            if state.closed || state.sockets.len() >= MAX_FLOWS || owner.cancel.is_cancelled() {
                return Err("WebSocket opening cancelled".into());
            }
            state.sockets.insert(id, slot);
        }
        let runtime = self.web.clone();
        tasks.spawn(async move {
            let result = tokio::select! {
                biased;
                _ = owner.cancel.cancelled() => Err("WebSocket cancelled".into()),
                result = runtime.socket_consumer(&owner, stream, outgoing, incoming) => result,
                _ = connection.closed() => Err("WebSocket peer closed before complete receipt".into()),
            };
            *outcome.lock().unwrap() = Some(result.is_ok());
            if result.is_ok() {
                runtime.completed(&owner);
            }
        });
        Ok(
            json!({"id":id.to_string(),"headers":ready.headers,"max_message_bytes":web::MAX_SOCKET_MESSAGE}),
        )
    }
    pub async fn send_web_socket(
        &self,
        id: &str,
        message: WebSocketMessage,
    ) -> Result<(), ClientError> {
        message.validate()?;
        // Validate close codes before enqueueing, so an invalid caller value
        // cannot panic the upstream WebSocket library or end another flow.
        let _ = target_message(message.clone())?;
        let slot = self.web.socket(id)?;
        tokio::select! {
            biased;
            _ = slot.cancel.cancelled() => Err("WebSocket ended".into()),
            result = slot.outgoing.send(message) => result.map_err(|_| "WebSocket ended".into()),
        }
    }
    pub async fn read_web_socket(&self, id: &str, timeout: Duration) -> Result<Value, ClientError> {
        if timeout > Duration::from_secs(5) {
            return Err("WebSocket read wait too long".into());
        }
        let slot = self.web.socket(id)?;
        let mut reader = slot.incoming.lock().await;
        match tokio::time::timeout(timeout, reader.recv()).await {
            Err(_) => Ok(json!({"pending":true,"closed":false})),
            Ok(Some(message)) => {
                Ok(json!({"pending":false,"closed":message.is_close(),"message":message}))
            }
            Ok(None) => Ok(
                json!({"pending":false,"closed":true,"failed":*slot.outcome.lock().unwrap() != Some(true)}),
            ),
        }
    }
    pub async fn close_web_socket(&self, id: &str) -> Result<Value, ClientError> {
        let slot = self.web.socket(id)?;
        let identifier = id.parse::<u64>()?;
        let _close_owner = slot.cancel.clone().drop_guard();
        // The protocol budget includes waiting for outgoing queue capacity.
        // Cancellation has a separate bounded join of this exact owner.
        let protocol = tokio::time::timeout(Duration::from_secs(5), async {
            if !slot.cancel.is_cancelled() {
                let _ = self
                    .send_web_socket(
                        id,
                        WebSocketMessage::Close {
                            code: 1000,
                            reason: String::new(),
                        },
                    )
                    .await;
            }
            self.web.socket_joined(identifier).await;
        })
        .await;
        if protocol.is_err() {
            slot.cancel.cancel();
        }
        let joined =
            tokio::time::timeout(Duration::from_secs(1), self.web.socket_joined(identifier)).await;
        self.web.state.lock().unwrap().sockets.remove(&identifier);
        if joined.is_err() {
            return Err("WebSocket cancelled owner did not finish".into());
        }
        if protocol.is_err() {
            return Err("WebSocket close handshake did not finish".into());
        }
        Ok(json!({"closed":true,"failed":*slot.outcome.lock().unwrap() != Some(true)}))
    }
}
