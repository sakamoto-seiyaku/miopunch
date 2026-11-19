use std::{
    future::Future,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures_util::FutureExt;
use iroh::endpoint::ConnectionError;
use n0_future::time::Instant;
use serde_json::{Value, json};

use super::AttemptRole;

/// Local clock shared by connection diagnostics in this process.
pub fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Fixed transport reason categories. In particular, never log peer close bytes.
pub fn close_reason(reason: Option<&ConnectionError>) -> &'static str {
    match reason {
        None => "open",
        Some(ConnectionError::ApplicationClosed(_)) => "peer_application_close",
        Some(ConnectionError::ConnectionClosed(_)) => "peer_transport_close",
        Some(ConnectionError::Reset) => "peer_reset",
        Some(ConnectionError::TimedOut) => "transport_timeout",
        Some(ConnectionError::LocallyClosed) => "locally_closed",
        Some(_) => "transport_error",
    }
}

/// Attribution available from transport alone. LocallyClosed carries no owner.
pub fn close_origin(reason: Option<&ConnectionError>) -> &'static str {
    match reason {
        Some(ConnectionError::ApplicationClosed(_) | ConnectionError::ConnectionClosed(_)) => {
            "peer"
        }
        Some(ConnectionError::TimedOut | ConnectionError::Reset) => "transport",
        _ => "unknown",
    }
}

pub(crate) fn emit(report: Value) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        // Keep a whole record together when stdout and stderr share a pipe.
        let _ = std::io::stderr()
            .lock()
            .write_all(format!("{report}\n").as_bytes());
    }
    #[cfg(target_arch = "wasm32")]
    {
        #[wasm_bindgen::prelude::wasm_bindgen]
        extern "C" {
            #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = console, js_name = debug)]
            fn debug(message: &str);
        }
        debug(&report.to_string());
    }
}

fn next_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AttemptLog {
    pub(crate) local_id: usize,
    pub(crate) route_phase_id: Option<usize>,
    shared_id: Option<[u8; 16]>,
    round: Option<u16>,
    pub(crate) original: Option<usize>,
    pub(crate) control: Option<usize>,
    role: AttemptRole,
    source: &'static str,
}

impl AttemptLog {
    pub(crate) fn new(role: AttemptRole, source: &'static str) -> Self {
        Self {
            local_id: next_id(),
            route_phase_id: None,
            shared_id: None,
            round: None,
            original: None,
            control: None,
            role,
            source,
        }
    }

    pub(crate) fn for_dial(role: AttemptRole, dial: Option<(usize, usize)>) -> Self {
        let mut log = Self::new(role, "connect");
        if let Some((attempt, route)) = dial {
            log.local_id = attempt;
            log.route_phase_id = Some(route);
        }
        log
    }

    pub(crate) fn with_shared(mut self, id: [u8; 16]) -> Self {
        self.shared_id = Some(id);
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn with_round(mut self, round: u16) -> Self {
        self.round = Some(round);
        self
    }

    fn report(self, phase: &'static str, result: &'static str, duration_ms: u64, detail: Value) {
        let mut report = json!({
            "event":"connection-timeline","monotonicMs":monotonic_ms(),
            "localAttemptId":self.local_id,"routePhaseId":self.route_phase_id,"attemptId":self.shared_id.map(hex::encode),
            "stableId":self.original,"controlStableId":self.control,
            "role":match self.role { AttemptRole::Initiator => "initiator", AttemptRole::Responder => "responder" },
            "source":self.source,"phase":phase,"result":result,"durationMs":duration_ms,
            "detail":detail,
        });
        if let Some(round) = self.round {
            report["round"] = json!(round);
        }
        emit(report);
    }

    pub(crate) fn event(self, phase: &'static str, result: &'static str, detail: Value) {
        self.report(phase, result, 0, detail);
    }

    pub(crate) fn connection_ended(
        self,
        reason: &ConnectionError,
        local_close_owner: Option<&'static str>,
    ) {
        let owner = if matches!(reason, ConnectionError::LocallyClosed) {
            local_close_owner.unwrap_or("unknown")
        } else {
            close_origin(Some(reason))
        };
        self.event(
            "connection-ended",
            "closed",
            json!({
                "owner":owner,"reason":close_reason(Some(reason)),
            }),
        );
    }

    pub(crate) fn phase(self, name: &'static str, detail: Value) -> Phase {
        let start = Instant::now();
        let id = next_id();
        self.report(name, "started", 0, json!({"phaseId":id,"input":detail}));
        Phase {
            log: self,
            name,
            start,
            id,
            finished: false,
        }
    }
}

pub(crate) struct Phase {
    log: AttemptLog,
    name: &'static str,
    start: Instant,
    id: usize,
    finished: bool,
}

impl Phase {
    pub(crate) fn id(&self) -> usize {
        self.id
    }
    pub(crate) fn finish(self, result: &'static str) {
        self.finish_with(result, Value::Null);
    }

    pub(crate) fn finish_with(mut self, result: &'static str, detail: Value) {
        self.finished = true;
        self.report(result, detail);
    }

    pub(crate) fn run<T, E>(
        self,
        work: impl Future<Output = Result<T, E>>,
    ) -> impl Future<Output = Result<T, E>> {
        work.inspect(move |result| {
            self.finish(if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            });
        })
    }

    fn report(&self, result: &'static str, detail: Value) {
        self.log.report(
            self.name,
            result,
            self.start.elapsed().as_millis() as u64,
            json!({"phaseId":self.id,"output":detail}),
        );
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        if !self.finished {
            self.report("cancelled", Value::Null);
        }
    }
}
