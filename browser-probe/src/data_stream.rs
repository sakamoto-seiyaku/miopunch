//! Bounded WASM stream operations for hosts and later application modules.
use super::*;
use crate::data_stream::{CHUNK_BYTES, DataReader, DataStream, DataWriter};
use futures_util::FutureExt;

#[wasm_bindgen]
pub struct BrowserDataStream {
    writer: std::rc::Rc<tokio::sync::Mutex<DataWriter>>,
    reader: std::rc::Rc<tokio::sync::Mutex<DataReader>>,
    connection: Connection,
    gate: BrowserGate,
    cancel: tokio_util::sync::CancellationToken,
}

#[wasm_bindgen]
impl BrowserMemberConnection {
    pub async fn open_data_stream(&self) -> Result<BrowserDataStream, JsValue> {
        self.qualified_snapshot().await?;
        Ok(BrowserDataStream::new(
            DataStream::open(&self.connection).await.map_err(error)?,
            self.connection.clone(),
            self.gate.clone(),
        ))
    }
    pub async fn accept_data_stream(&self) -> Result<BrowserDataStream, JsValue> {
        self.qualified_snapshot().await?;
        Ok(BrowserDataStream::new(
            DataStream::accept(&self.connection).await.map_err(error)?,
            self.connection.clone(),
            self.gate.clone(),
        ))
    }
}

impl BrowserDataStream {
    fn new(stream: DataStream, connection: Connection, gate: BrowserGate) -> Self {
        let (writer, reader) = stream.split();
        Self {
            writer: std::rc::Rc::new(tokio::sync::Mutex::new(writer)),
            reader: std::rc::Rc::new(tokio::sync::Mutex::new(reader)),
            connection,
            gate,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }
}

#[wasm_bindgen]
impl BrowserDataStream {
    pub async fn write_chunk(&self, bytes: js_sys::Uint8Array) -> Result<(), JsValue> {
        if bytes.length() as usize > CHUNK_BYTES || self.cancel.is_cancelled() {
            return Err(rejected());
        }
        let mut writer = self
            .writer
            .try_lock()
            .map_err(|_| error("stream write busy"))?;
        let work = async {
            self.gate.qualified_snapshot(&self.connection).await?;
            writer.write_chunk(&bytes.to_vec()).await.map_err(error)
        }
        .fuse();
        let cancelled = self.cancel.cancelled().fuse();
        futures_util::pin_mut!(work, cancelled);
        futures_util::select_biased! { _ = cancelled => Err(error("stream cancelled")), result = work => result }
    }
    pub async fn read_chunk(&self) -> Result<Vec<u8>, JsValue> {
        let mut reader = self
            .reader
            .try_lock()
            .map_err(|_| error("stream read busy"))?;
        let work = async {
            self.gate.qualified_snapshot(&self.connection).await?;
            reader.read_chunk().await.map_err(error)
        }
        .fuse();
        let cancelled = self.cancel.cancelled().fuse();
        futures_util::pin_mut!(work, cancelled);
        futures_util::select_biased! { _ = cancelled => Err(error("stream cancelled")), result = work => result }
    }
    pub async fn finish(&self) -> Result<(), JsValue> {
        if self.cancel.is_cancelled() {
            return Err(error("stream cancelled"));
        }
        let mut writer = self
            .writer
            .try_lock()
            .map_err(|_| error("stream write busy"))?;
        self.gate.qualified_snapshot(&self.connection).await?;
        writer.finish().map_err(error)
    }
    pub fn cancel(&self) {
        if self.cancel.is_cancelled() {
            return;
        }
        self.cancel.cancel();
        let writer = self.writer.clone();
        let reader = self.reader.clone();
        wasm_bindgen_futures::spawn_local(async move {
            writer.lock().await.cancel();
            reader.lock().await.cancel();
        });
    }
}
impl Drop for BrowserDataStream {
    fn drop(&mut self) {
        self.cancel();
    }
}
