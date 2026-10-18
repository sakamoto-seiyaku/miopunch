//! One-shot, owned diagnostic holds at existing RTC Control boundaries.
//! Never installed by default and never a permission source.
use crate::control_protocol::AuthenticatedControl;
use crate::{
    rtc_association::RtcDeadline,
    rtc_transport::{RtcDatagramIo, RtcRouteLease},
};
use futures_util::FutureExt;
use iroh::endpoint::WeakControlConnection;
use serde_json::{Value, json};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlCheckpointStage {
    ChannelReady,
    BeforeRelease,
}

struct Observation {
    metadata: Value,
    io: RtcDatagramIo,
    original: WeakControlConnection,
    held: Arc<AtomicBool>,
    release: CancellationToken,
    actual_sends: Arc<dyn Fn() -> Value + Send + Sync>,
    release_probe: Option<Value>,
    cleanup_details: Option<Value>,
}

pub(crate) struct ControlFixture {
    stage: ControlCheckpointStage,
    observation: Mutex<Option<Observation>>,
    hold_cleanup: bool,
    cleanup_held: Arc<AtomicBool>,
    cleanup_release: CancellationToken,
}
impl ControlFixture {
    pub(crate) fn new(stage: ControlCheckpointStage, hold_cleanup: bool) -> Self {
        Self {
            stage,
            observation: Mutex::new(None),
            hold_cleanup,
            cleanup_held: Arc::new(AtomicBool::new(false)),
            cleanup_release: CancellationToken::new(),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Value> {
        let observation = self.observation.lock().unwrap();
        let observation = observation.as_ref()?;
        Some(json!({
            "stage":self.stage,"metadata":observation.metadata,
            "held":observation.held.load(Ordering::Acquire),
            "originalOpen":observation.original.is_open(),
            "route":observation.io.report(),"dataChannel":(observation.actual_sends)(),
            "releaseProbe":observation.release_probe,
            "cleanupHeld":self.cleanup_held.load(Ordering::Acquire),
            "cleanupDetails":observation.cleanup_details,
        }))
    }

    pub(crate) fn holds_cleanup(&self) -> bool {
        self.hold_cleanup
    }

    pub(crate) fn record_cleanup_details(&self, original: usize, details: Value) {
        let mut observation = self.observation.lock().unwrap();
        if let Some(observation) = observation.as_mut()
            && observation.io.report().original_stable_id == original
        {
            observation.cleanup_details = Some(details);
        }
    }

    pub(crate) fn record_release_probe(&self, original: usize, value: Value) {
        let mut observation = self.observation.lock().unwrap();
        if let Some(observation) = observation.as_mut()
            && observation.io.report().original_stable_id == original
        {
            observation.release_probe = Some(value);
        }
    }

    /// Hold only the actual PC cleanup after the exact lease/pump is already
    /// retired. This lets a deterministic fault keep real SCTP open without
    /// permitting any original QUIC input, output or application release.
    pub(crate) async fn before_pc_close(&self, original: usize, budget: RtcDeadline) {
        if !self.hold_cleanup {
            return;
        }
        let applicable = self
            .observation
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|observation| {
                observation.io.report().original_stable_id == original
                    && !observation.io.is_active()
            });
        if !applicable {
            return;
        }
        self.cleanup_held.store(true, Ordering::Release);
        let _held = Held(self.cleanup_held.clone());
        // The original setup clock still owns this hold. A cancelled/dropped
        // enclosing hook's existing RAII closes the PC; no extra wait is added.
        let _ = n0_future::time::timeout(
            budget.remaining().unwrap_or_default(),
            self.cleanup_release.cancelled(),
        )
        .await;
    }

    pub(crate) fn release_cleanup(&self, attempt: &str) -> io::Result<()> {
        if !self
            .observation
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|observation| observation.metadata["attempt"].as_str() == Some(attempt))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "owned cleanup attempt mismatch",
            ));
        }
        self.cleanup_release.cancel();
        Ok(())
    }

    pub(crate) fn release(&self, attempt: &str) -> io::Result<()> {
        let observation = self.observation.lock().unwrap();
        let observation = observation.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "owned Control checkpoint not reached",
            )
        })?;
        if observation.metadata["attempt"].as_str() != Some(attempt)
            || !observation.io.is_active()
            || !observation.original.is_open()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "owned Control checkpoint attempt retired or mismatched",
            ));
        }
        observation.release.cancel();
        Ok(())
    }

    pub(crate) fn clear(&self) {
        self.cleanup_release.cancel();
        if let Some(observation) = self.observation.lock().unwrap().as_ref() {
            // Clearing a diagnostic cannot grant data or let a held operation
            // proceed. Its existing owner handles the retired lease normally.
            observation.io.retire();
            observation.release.cancel();
        }
    }

    pub(crate) async fn checkpoint(
        &self,
        stage: ControlCheckpointStage,
        original: &(impl AuthenticatedControl + ?Sized),
        lease: &RtcRouteLease,
        budget: RtcDeadline,
        metadata: Value,
        actual_sends: Arc<dyn Fn() -> Value + Send + Sync>,
    ) -> io::Result<()> {
        if stage != self.stage {
            return Ok(());
        }
        let release = CancellationToken::new();
        let held = Arc::new(AtomicBool::new(true));
        let io = lease.io();
        {
            let mut observation = self.observation.lock().unwrap();
            if observation.is_some() {
                return Ok(()); // One diagnostic hold, never the subsequent M.
            }
            *observation = Some(Observation {
                metadata,
                io: io.clone(),
                original: original.revocation_handle(),
                held: held.clone(),
                release: release.clone(),
                actual_sends,
                release_probe: None,
                cleanup_details: None,
            });
        }
        let _held = Held(held);
        #[cfg(not(target_arch = "wasm32"))]
        println!(
            "{}",
            json!({"event":"rtc-control-checkpoint","checkpoint":self.snapshot()})
        );
        let cancellation = original.attempt_cancellation().unwrap_or_default();
        let wait = async {
            let cancelled = cancellation.cancelled().fuse();
            let released = release.cancelled().fuse();
            futures_util::pin_mut!(cancelled, released);
            futures_util::select_biased! {
                _ = cancelled => Err(io::Error::new(io::ErrorKind::Interrupted,"original member attempt cancelled")),
                _ = released => Ok(()),
            }
        };
        n0_future::time::timeout(budget.remaining()?, io.while_active(wait))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "original Control caller deadline")
            })??
    }
}
impl Drop for ControlFixture {
    fn drop(&mut self) {
        self.clear();
    }
}
struct Held(Arc<AtomicBool>);
impl Drop for Held {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A Native fixture scheduling pause after an authenticated C stream has
/// supplied an actual matching Ready, before that read completes to the
/// coordinator. The original bytes stay on the read future's stack. This
/// neither fabricates Ready nor grants a candidate permission.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct CommonReadyInputFixture {
    original: usize,
    observation: Mutex<Option<Value>>,
    release: CancellationToken,
}

#[cfg(not(target_arch = "wasm32"))]
impl CommonReadyInputFixture {
    pub(crate) fn new(original: usize) -> Self {
        Self {
            original,
            observation: Mutex::new(None),
            release: CancellationToken::new(),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Value> {
        self.observation.lock().unwrap().clone()
    }

    pub(crate) fn release(&self) {
        // Scheduling release only. The coordinator still performs its actual
        // authorization, revision and PreparedPath checks after this read.
        if let Some(observation) = self.observation.lock().unwrap().as_mut() {
            observation["releaseRequestedMonotonicMs"] =
                json!(crate::rtc_transport::diagnostic_monotonic_ms());
        }
        self.release.cancel();
    }

    pub(crate) async fn after_actual_ready_input(
        &self,
        original: usize,
        control: usize,
        stream: String,
        actual_frame: &[u8],
        deadline: n0_future::time::Instant,
    ) -> io::Result<()> {
        if original != self.original || self.release.is_cancelled() {
            return Ok(());
        }
        {
            let mut observation = self.observation.lock().unwrap();
            if observation.is_some() {
                return Ok(());
            }
            *observation = Some(json!({"version":1,"held":true,
                "originalStableId":original,"controlStableId":control,
                "streamId":stream,"kind":actual_frame[5],"frameLength":actual_frame.len(),
                "frameBlake3":blake3::hash(actual_frame).to_hex().to_string(),
                "attemptBlake3":blake3::hash(&actual_frame[6..22]).to_hex().to_string(),
                "actualAuthenticatedRead":true,"attemptMatched":true,
                "physicalArrivalAfterInvalidation":false,
                "readCompletionHeldMonotonicMs":crate::rtc_transport::diagnostic_monotonic_ms()}));
        }
        println!(
            "{}",
            json!({"event":"actual-common-ready-input-held","observation":self.snapshot()})
        );
        let _held = CommonReadyInputHeld(&self.observation);
        let remaining = deadline
            .saturating_duration_since(n0_future::time::Instant::now())
            .min(std::time::Duration::from_secs(5));
        n0_future::time::timeout(remaining, self.release.cancelled())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned Common Ready read scheduling deadline",
                )
            })?;
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CommonReadyInputFixture {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct CommonReadyInputHeld<'a>(&'a Mutex<Option<Value>>);
#[cfg(not(target_arch = "wasm32"))]
impl Drop for CommonReadyInputHeld<'_> {
    fn drop(&mut self) {
        if let Some(observation) = self.0.lock().unwrap().as_mut() {
            observation["held"] = json!(false);
            observation["readWaitEndedMonotonicMs"] =
                json!(crate::rtc_transport::diagnostic_monotonic_ms());
        }
    }
}
