//! Opt-in private packet provenance for the owned Native integration fixture.
//! Encoder declarations are matched to actual payload, encrypted packet slices,
//! the final coalesced transmit and the real pump packet. No record grants data.
use noq_proto::packet_witness::{self, Record, Value as WitnessValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
};

const PROOF_LIMIT: usize = 512;
const FILE_LIMIT: usize = 24 * 1024 * 1024;
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
type PacketKey = (u64, u64, String, u64);
#[derive(Default)]
struct Proofs {
    sequence: usize,
    declared: BTreeMap<(u64, u64), Value>,
    payloads: BTreeMap<PacketKey, Value>,
    encrypted: BTreeMap<(u64, String), Value>,
    datagrams: BTreeMap<(u64, String), Value>,
    arm: Option<(u64, usize)>,
}
impl Proofs {
    fn observe(
        &mut self,
        sequence: usize,
        stable: Option<u64>,
        stage: &str,
        f: &Value,
    ) -> Result<(), ()> {
        self.sequence = self.sequence.max(sequence);
        let number = |name: &str| f[name].as_u64().ok_or(());
        let text = |name: &str| f[name].as_str().map(str::to_owned).ok_or(());
        match stage {
            "ordinary-stream-declaration" => {
                let stable = number("original_stable_id")?;
                self.declared.insert((stable,number("stream_id")?),json!({"sequence":sequence,
                    "payloadLength":number("payload_len")?,"payloadSha256":text("payload_sha256")?}));
            }
            "actual-payload-appended" if f["class"] == "STREAM" && f["offset"] == 0 => {
                let stable = stable.ok_or(())?;
                let stream = number("stream_id")?;
                if let Some(declared) = self.declared.get(&(stable, stream))
                    && declared["payloadLength"] == f["payload_len"]
                    && declared["payloadSha256"] == f["payload_sha256"]
                {
                    let key = (
                        stable,
                        number("path_id")?,
                        text("space")?,
                        number("packet_number")?,
                    );
                    self.payloads.insert(key,json!({"declaration":declared,"streamId":stream,
                        "offset":0,"payloadLength":number("payload_len")?,"payloadSha256":text("payload_sha256")?,
                        "appendedSequence":sequence}));
                }
            }
            "actual-packet-encrypted" => {
                let stable = stable.ok_or(())?;
                let key = (
                    stable,
                    number("path_id")?,
                    text("space")?,
                    number("packet_number")?,
                );
                if let Some(payload) = self.payloads.get(&key) {
                    self.encrypted.insert(
                        (stable, text("ciphertext_sha256")?),
                        json!({"payload":payload,
                        "packet":f,"encryptedSequence":sequence}),
                    );
                }
            }
            "before-actual-carrier-call" => {} // Raw transmit handled before serialization below.
            "private-transmit-bound-exceeded" => return Err(()),
            _ => {}
        }
        if [
            self.declared.len(),
            self.payloads.len(),
            self.encrypted.len(),
            self.datagrams.len(),
        ]
        .into_iter()
        .any(|len| len > PROOF_LIMIT)
        {
            return Err(());
        }
        Ok(())
    }
    fn transmit(
        &mut self,
        stable: u64,
        sequence: usize,
        f: &Value,
        actual: &[u8],
    ) -> Result<(), ()> {
        let segment = usize::try_from(f["segment_size"].as_u64().ok_or(())?).map_err(|_| ())?;
        if segment == 0
            || actual.len()
                != usize::try_from(f["transmit_len"].as_u64().ok_or(())?).map_err(|_| ())?
        {
            return Err(());
        }
        for ((original, digest), proof) in &self.encrypted {
            if *original != stable {
                continue;
            }
            let p = &proof["packet"];
            let get = |name: &str| {
                p[name]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(())
            };
            let start = get("packet_offset")?;
            let end = start.checked_add(get("packet_len")?).ok_or(())?;
            if end > actual.len() || sha(&actual[start..end]) != *digest {
                continue;
            }
            let datagram = get("datagram_offset")?;
            if datagram % segment != 0
                || datagram.checked_div(segment) != Some(get("datagram_index")?)
                || start < datagram
            {
                return Err(());
            }
            let datagram_end = datagram.checked_add(segment).ok_or(())?.min(actual.len());
            if end > datagram_end {
                return Err(());
            }
            let full_digest = sha(&actual[datagram..datagram_end]);
            let value = json!({"proof":proof,"datagramOffset":datagram,"datagramLength":datagram_end-datagram,
                "actualTransmitSegmentSize":segment,"actualTransmitLength":actual.len(),
                "ciphertextSha256":full_digest,"actualBeforeCarrierSequence":sequence});
            self.datagrams.insert((stable, full_digest), value);
        }
        if self.datagrams.len() > PROOF_LIMIT {
            return Err(());
        }
        Ok(())
    }
}

enum WriterMessage {
    Record(Value),
    Finish,
}
fn observe_record(
    record: Record,
    proofs: &Mutex<Proofs>,
    lost: &AtomicUsize,
    sender: &SyncSender<WriterMessage>,
) {
    let mut f = serde_json::Map::new();
    let mut actual_transmit = None;
    for (name, value) in record.fields {
        let value = match value {
            WitnessValue::U64(value) => json!(value),
            WitnessValue::Bool(value) => json!(value),
            WitnessValue::Text(value) => json!(value),
            WitnessValue::Bytes(value) => {
                if name != "ciphertext_exact" {
                    lost.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                actual_transmit = Some(value);
                Value::Null
            }
        };
        if f.insert(name.to_owned(), value).is_some() {
            lost.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    let f = Value::Object(f);
    let stable = record.connection_stable_id.map(|v| v as u64);
    {
        // This private metadata lock has no I/O, SDK calls, or await while
        // held. Contention must preserve the real event, never discard it.
        let Ok(mut state) = proofs.lock() else {
            lost.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let result = state
            .observe(record.sequence, stable, record.stage, &f)
            .and_then(|()| {
                if record.stage == "before-actual-carrier-call" {
                    state.transmit(
                        stable.ok_or(())?,
                        record.sequence,
                        &f,
                        actual_transmit.as_deref().ok_or(())?,
                    )
                } else {
                    Ok(())
                }
            });
        if result.is_err() {
            lost.fetch_add(1, Ordering::Relaxed);
        }
    }
    let value = json!({"sequence":record.sequence,"connectionStableId":stable,"stage":record.stage,
        "fields":f,"actualTransmitCiphertextHex":actual_transmit.map(hex::encode)});
    if sender.try_send(WriterMessage::Record(value)).is_err() {
        lost.fetch_add(1, Ordering::Relaxed);
    }
}

/// Process fixture capture, with a bounded writer separate from QUIC's lock.
pub struct PrivatePacketCaptureFixture {
    proofs: Arc<Mutex<Proofs>>,
    lost: Arc<AtomicUsize>,
    sender: SyncSender<WriterMessage>,
    writer: Option<JoinHandle<io::Result<usize>>>,
}
impl PrivatePacketCaptureFixture {
    pub fn start(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "private Native packet fixture requires Unix file modes",
            ));
        }
        let mut file = options.open(path)?;
        let (sender, receiver) = mpsc::sync_channel::<WriterMessage>(1024);
        let lost = Arc::new(AtomicUsize::new(0));
        let proofs = Arc::new(Mutex::new(Proofs::default()));
        let callback_lost = lost.clone();
        let callback_proofs = proofs.clone();
        let callback_sender = sender.clone();
        packet_witness::install(move |record: Record| {
            observe_record(record, &callback_proofs, &callback_lost, &callback_sender);
        })?;
        let writer = std::thread::spawn(move || {
            let mut bytes = 0usize;
            while let Ok(message) = receiver.recv() {
                match message {
                    WriterMessage::Finish => break,
                    WriterMessage::Record(value) => {
                        let line = serde_json::to_vec(&value)?;
                        bytes = bytes.checked_add(line.len() + 1).ok_or_else(|| {
                            io::Error::other("private packet byte count overflow")
                        })?;
                        if bytes > FILE_LIMIT {
                            return Err(io::Error::other("private packet capture byte bound"));
                        }
                        file.write_all(&line)?;
                        file.write_all(b"\n")?;
                    }
                }
            }
            file.flush()?;
            Ok(bytes)
        });
        Ok(Self {
            proofs,
            lost,
            sender,
            writer: Some(writer),
        })
    }
    /// Arm only fresh ordinary declarations; old PONG retransmissions cannot match.
    pub fn matcher(&self, stable: usize) -> io::Result<Arc<dyn Fn(&[u8]) -> bool + Send + Sync>> {
        {
            let mut state = self.proofs.lock().unwrap();
            if state.arm.is_some() {
                return Err(io::Error::other("packet witness matcher already armed"));
            }
            let floor = state.sequence;
            state.arm = Some((stable as u64, floor));
        }
        let state = self.proofs.clone();
        let lost = self.lost.clone();
        Ok(Arc::new(move |actual: &[u8]| {
            let digest = sha(actual);
            let Ok(state) = state.lock() else {
                lost.fetch_add(1, Ordering::Relaxed);
                return false;
            };
            let Some((expected, floor)) = state.arm else {
                return false;
            };
            state.datagrams.get(&(expected, digest)).is_some_and(|p| {
                p["proof"]["payload"]["declaration"]["sequence"]
                    .as_u64()
                    .is_some_and(|n| n > floor as u64)
            })
        }))
    }
    pub fn proof(&self, stable: usize, digest: &str) -> Option<Value> {
        self.proofs
            .lock()
            .unwrap()
            .datagrams
            .get(&(stable as u64, digest.to_owned()))
            .cloned()
    }
    /// Finish only after all SDK endpoints and pump producers have naturally ended.
    pub fn finish(&mut self) -> io::Result<Value> {
        let status = packet_witness::finish();
        self.sender
            .send(WriterMessage::Finish)
            .map_err(|_| io::Error::other("private packet writer ended before finish"))?;
        let bytes = self
            .writer
            .take()
            .ok_or_else(|| io::Error::other("private packet capture already finished"))?
            .join()
            .map_err(|_| io::Error::other("private packet writer panicked"))??;
        Ok(
            json!({"records":status.records,"reservedBytes":status.bytes,"adapterLost":status.lost,
            "writerBytes":bytes,"observerLost":self.lost.load(Ordering::Acquire),"writerNaturallyJoined":true}),
        )
    }
}
impl Drop for PrivatePacketCaptureFixture {
    fn drop(&mut self) {
        if self.writer.is_some() {
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A concurrent SDK drive encounters the same metadata lock held by fixture
    // observation. Every real record must survive that contention.
    #[test]
    fn concurrent_drive_record_survives_fixture_metadata_contention() {
        let proofs = Arc::new(Mutex::new(Proofs::default()));
        let lost = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::sync_channel(4);
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let held = proofs.lock().unwrap();
        let task_proofs = proofs.clone();
        let task_lost = lost.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            observe_record(
                Record {
                    sequence: 1,
                    connection_stable_id: Some(11),
                    stage: "builder-open",
                    fields: vec![],
                },
                &task_proofs,
                &task_lost,
                &sender,
            );
            done_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        // Ensure the contending callback gets a chance to run before release.
        let _ = done_rx.recv_timeout(std::time::Duration::from_millis(25));
        drop(held);
        worker.join().unwrap();
        assert_eq!(
            lost.load(Ordering::Acquire),
            0,
            "contention lost an actual record"
        );
        let WriterMessage::Record(record) = receiver.try_recv().expect("actual record retained")
        else {
            panic!("unexpected finish");
        };
        assert_eq!(record["sequence"], 1);
        assert_eq!(record["stage"], "builder-open");
        assert!(receiver.try_recv().is_err(), "record duplicated");
    }
}
