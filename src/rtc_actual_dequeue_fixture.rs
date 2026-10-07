//! Native rtc-probe fixture only: pause one real matcher-qualified pending packet.
//! No packets, capacity failures, permissions or business provenance are invented.
//! Snapshot identifiers/digests are bounded PRIVATE diagnostics, not public output.
use crate::rtc_transport::{MAX_DATAGRAM, RtcDatagramIo, diagnostic_monotonic_ms};
use iroh_base::CustomAddr;
use n0_future::time::Instant;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct ActualDequeueBarrier {
    expected_original: usize,
    expected_local: CustomAddr,
    expected_resource_revision: u64,
    expected_path: String,
    deadline: Instant,
    // Root supplies a read-only predicate over bounded private encoder state.
    // Fixture metadata synchronization must not call SDK/transport APIs or await;
    // never capture M/PC/lease/transport or guess ordinary ciphertext provenance.
    // This callback runs without registry/fixture state locks held.
    matcher: Arc<dyn Fn(&[u8]) -> bool + Send + Sync>,
    observed: Mutex<Value>,
    released: CancellationToken,
}

impl ActualDequeueBarrier {
    pub(crate) fn new(
        expected_original: usize,
        expected_local: CustomAddr,
        expected_resource_revision: u64,
        expected_path: String,
        remaining_deadline: Instant,
        matcher: Arc<dyn Fn(&[u8]) -> bool + Send + Sync>,
    ) -> io::Result<Self> {
        let now = Instant::now();
        if remaining_deadline <= now || expected_path.is_empty() || expected_path.len() > 128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "owned actual dequeue fixture needs a live bounded remaining budget/path",
            ));
        }
        let deadline = remaining_deadline.min(now + Duration::from_secs(5));
        let armed_at = diagnostic_monotonic_ms();
        let observed = json!({"version":1,"matched":false,"waiting":false,
            "originalStableId":expected_original,"selectedPathIdAtArm":expected_path,
            "armedMonotonicMs":armed_at,"maximumHoldMs":5000,
            "remainingAtArmMs":deadline.saturating_duration_since(now).as_millis(),
            "skippedNonmatchingPackets":0,"businessProvenance":"UNOBSERVED",
            "schedulerFixture":true,"naturalCapacityPressureClaimed":false});
        Ok(Self {
            expected_original,
            expected_local,
            expected_resource_revision,
            expected_path,
            deadline,
            matcher,
            observed: Mutex::new(observed),
            released: CancellationToken::new(),
        })
    }

    pub(crate) fn path_id(&self) -> &str {
        &self.expected_path
    }
    pub(crate) fn snapshot(&self) -> Value {
        self.observed.lock().unwrap().clone()
    }
    pub(crate) fn release_or_cleanup(&self) {
        // Only scheduler release. Existing while_active/try_send checks remain.
        self.released.cancel();
    }
    pub(crate) fn is_waiting(&self) -> bool {
        self.observed.lock().unwrap()["waiting"] == true
    }

    pub(crate) async fn after_actual_pending(
        &self,
        actual_io: &RtcDatagramIo,
        actual_local: &CustomAddr,
        actual_packet: &[u8],
    ) -> io::Result<()> {
        if self.released.is_cancelled() || self.observed.lock().unwrap()["matched"] == true {
            return Ok(());
        }
        let actual_route = actual_io.report();
        if actual_route.original_stable_id != self.expected_original
            || actual_local != &self.expected_local
            || actual_route.resource_revision != self.expected_resource_revision
            || !actual_route.active
            || !actual_route.qualified
            || actual_packet.is_empty()
            || actual_packet.len() > MAX_DATAGRAM
        {
            return Ok(());
        }
        // Actual plaintext/ciphertext provenance is Root's separate evidence.
        // Matcher false leaves the real packet untouched and does not suspend
        // ACK/control/probe. This is not a packet parser or qualification source.
        if !(self.matcher)(actual_packet) {
            let mut observed = self.observed.lock().unwrap();
            let count = observed["skippedNonmatchingPackets"].as_u64().unwrap_or(0);
            observed["skippedNonmatchingPackets"] = json!(count.saturating_add(1));
            return Ok(());
        }
        if self.released.is_cancelled() {
            return Ok(());
        }
        if self.deadline <= Instant::now() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "owned actual dequeue scheduler remaining deadline elapsed",
            ));
        }
        let digest = ciphertext_digest(actual_packet);
        {
            let mut observed = self.observed.lock().unwrap();
            if observed["matched"] == true {
                return Ok(());
            }
            let skipped = observed["skippedNonmatchingPackets"].clone();
            *observed = json!({"version":1,"matched":true,"waiting":true,
                "originalStableId":self.expected_original,"selectedPathIdAtArm":self.expected_path,
                "actualPacketLength":actual_packet.len(),"actualCiphertextSha256":digest,
                "actualRouteAtMatch":actual_route,"matcherQualified":true,
                "skippedNonmatchingPackets":skipped,"businessProvenance":"UNOBSERVED",
                "schedulerFixture":true,"naturalCapacityPressureClaimed":false,
                "waitStartedMonotonicMs":diagnostic_monotonic_ms()});
        }
        // This borrows the SAME real production pending Vec until its wait is
        // released/times out or enclosing while_active drops the work future.
        let mut waiting = Waiting {
            observed: &self.observed,
            actual_packet,
            reason: "wait-future-dropped",
        };
        let result = n0_future::time::timeout(
            self.deadline.saturating_duration_since(Instant::now()),
            self.released.cancelled(),
        )
        .await;
        waiting.reason = if result.is_ok() {
            "scheduler-released"
        } else {
            "fixture-deadline-elapsed"
        };
        drop(waiting);
        result.map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "owned actual dequeue scheduler barrier deadline",
            )
        })?;
        Ok(())
    }

    /// Synchronous identity witness immediately before actual outer pending
    /// drop. Does not copy bytes, send, classify or retain the pending Vec.
    pub(crate) fn observe_work_ended_pending(
        &self,
        actual_io: &RtcDatagramIo,
        actual_local: &CustomAddr,
        packet: Option<&[u8]>,
    ) {
        let route = actual_io.report();
        if route.original_stable_id != self.expected_original
            || actual_local != &self.expected_local
            || route.resource_revision != self.expected_resource_revision
        {
            return;
        }
        let length = packet.map(|bytes| bytes.len());
        let digest = packet.map(ciphertext_digest);
        let at = diagnostic_monotonic_ms();
        let mut observed = self.observed.lock().unwrap();
        let same = observed["matched"] == true
            && length == observed["actualPacketLength"].as_u64().map(|n| n as usize)
            && digest.as_deref() == observed["actualCiphertextSha256"].as_str();
        observed["workEnded"] = json!(true);
        observed["workEndedMonotonicMs"] = json!(at);
        observed["samePendingObservedBeforeImmediateDrop"] = json!(same);
        observed["terminalPendingLength"] = json!(length);
        observed["terminalPendingCiphertextSha256"] = json!(digest);
    }
}

fn ciphertext_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Waiting<'a> {
    observed: &'a Mutex<Value>,
    actual_packet: &'a [u8],
    reason: &'static str,
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        // Hash the real borrowed pending slot; only bounded digest/length kept.
        let digest = ciphertext_digest(self.actual_packet);
        let length = self.actual_packet.len();
        let at = diagnostic_monotonic_ms();
        let mut observed = self.observed.lock().unwrap();
        let same = observed["actualPacketLength"] == json!(length)
            && observed["actualCiphertextSha256"].as_str() == Some(digest.as_str());
        observed["waiting"] = json!(false);
        observed["waitFutureEndReason"] = json!(self.reason);
        observed["waitFutureEndedMonotonicMs"] = json!(at);
        observed["borrowedPendingIdentityMatchedAtWaitEnd"] = json!(same);
    }
}
