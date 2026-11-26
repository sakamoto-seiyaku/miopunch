//! Physical execution and business settlement of one agreed plan.

use std::{
    future::{Future, pending},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant as StdInstant,
};

use tokio::sync::{mpsc, oneshot};

use super::{
    ControlExchange, Duration, Instant, MemberUdpPlan, Planning, Recommendation, Reservation,
    Round, RoundTrace, StreamKind, denied, invalid, receive, send, time, wire,
};
use crate::{
    TransportAddr,
    endpoint::{
        LocalSocketBinding, LocalTransportAddr, Path, PathId, RecvStream, UdpCandidatePath,
        UdpSocketPathLease,
        nat_execution::{self, ExecutionTrace, PacketAuth, PhaseEvent, PhaseRun, Winner},
    },
};

/// Local observations of the most recent enhanced execution on this original.
#[derive(Debug, Clone)]
pub struct MemberUdpTraversalReport {
    /// Agreed round identifier.
    pub round: [u8; 16],
    /// Local original connection identifier.
    pub connection_id: usize,
    /// Local start time of execution, on the same monotonic clock as the ACK.
    pub started: StdInstant,
    /// Absolute local deadline from the agreed plan.
    pub deadline: Instant,
    /// Phase packet and socket accounting.
    pub trace: ExecutionTrace,
    /// The first authenticated match on an actual receiving socket.
    pub first_match: Option<(Winner, StdInstant)>,
    /// Both peers retained their raw winner and released physical mapping exclusivity.
    pub physical_released: Option<StdInstant>,
    /// Actual extra listeners bound by this round, including the winner if temporary.
    pub listeners: Vec<LocalSocketBinding>,
    /// Physical winner actually retained provisionally for the original.
    pub winner: Option<Winner>,
    /// Exact original path and fresh ACK after the raw traversal matched.
    pub validated: Option<(PathId, noq_proto::PathAcknowledgment)>,
    /// The business stream and local time when both ends confirmed the exchange.
    pub business: Option<(noq::StreamId, StdInstant)>,
    /// Whether the choosing endpoint scored its single recommendation.
    pub learned: bool,
    /// Local terminal result. Automatic success retains this end's validated path.
    /// Only `business` records an exchange confirmed on the selected path.
    pub outcome: Option<Result<(), io::ErrorKind>>,
    /// Local time of terminal cleanup.
    pub finished: Option<StdInstant>,
}

#[derive(Debug)]
pub(super) struct Execution {
    report: Mutex<MemberUdpTraversalReport>,
    trace: Mutex<ExecutionTrace>,
    business: mpsc::Sender<Business>,
}

#[derive(Debug)]
struct Business {
    stream: noq::StreamId,
    ready: oneshot::Sender<()>,
    confirmed: oneshot::Receiver<()>,
}

struct Running(Arc<Execution>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.update(|report| {
            report
                .outcome
                .get_or_insert(Err(io::ErrorKind::Interrupted));
            report.finished.get_or_insert_with(StdInstant::now);
        });
    }
}

impl Execution {
    fn update(&self, update: impl FnOnce(&mut MemberUdpTraversalReport)) {
        update(&mut self.report.lock().expect("member UDP execution"));
    }

    fn report(&self) -> MemberUdpTraversalReport {
        let mut report = self.report.lock().expect("member UDP execution").clone();
        report.trace = self.trace.lock().expect("member UDP trace").clone();
        report
    }
}

impl Planning {
    pub(in super::super) fn traversal(&self) -> Option<MemberUdpTraversalReport> {
        Some(
            self.traversal
                .lock()
                .expect("member UDP execution")
                .as_ref()?
                .report(),
        )
    }

    pub(super) async fn execute(
        &self,
        stream: &ControlExchange,
        plan: &MemberUdpPlan,
        recommendation: Option<Recommendation>,
        slot: &Reservation,
        kind: StreamKind,
        trace: &mut RoundTrace,
    ) -> io::Result<()> {
        trace.step("execution");
        trace.log(
            "plan",
            format_args!(
                " behavior={:?} fallback={} budget_ms={}",
                plan.id,
                plan.fallback,
                plan.execution_budget.as_millis()
            ),
        );
        let (business, incoming) = mpsc::channel(1);
        let state = Arc::new(Execution {
            report: Mutex::new(MemberUdpTraversalReport {
                round: plan.round,
                connection_id: self.connection.stable_id(),
                started: StdInstant::now(),
                deadline: plan.deadline,
                trace: ExecutionTrace::default(),
                first_match: None,
                physical_released: None,
                listeners: Vec::new(),
                winner: None,
                validated: None,
                business: None,
                learned: false,
                outcome: None,
                finished: None,
            }),
            trace: Mutex::new(ExecutionTrace::default()),
            business,
        });
        *self.traversal.lock().expect("member UDP execution") = Some(state.clone());
        let running = Running(state);
        let result = self
            .guarded(plan.deadline, &plan.scope, async {
                self.drive(
                    stream,
                    plan,
                    recommendation,
                    slot,
                    kind,
                    &running.0,
                    incoming,
                    trace,
                )
                .await
            })
            .await;
        running.0.update(|report| {
            report.outcome = Some(result.as_ref().map(|_| ()).map_err(io::Error::kind));
            report.finished = Some(StdInstant::now());
        });
        result
    }

    async fn drive(
        &self,
        stream: &ControlExchange,
        plan: &MemberUdpPlan,
        recommendation: Option<Recommendation>,
        slot: &Reservation,
        kind: StreamKind,
        state: &Execution,
        mut incoming: mpsc::Receiver<Business>,
        trace: &mut RoundTrace,
    ) -> io::Result<()> {
        if plan.disabled {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "UDP plan has no candidates",
            ));
        }
        let round = Round(plan.round);
        let peer = stream.send_control().await?;
        let check = || {
            self.check_scope(&plan.scope)?;
            if state
                .report
                .lock()
                .expect("member UDP report")
                .physical_released
                .is_none()
            {
                plan.evidence.check()?;
            }
            super::remaining(plan.deadline)?;
            Ok(())
        };
        let auth = PacketAuth {
            key: {
                let mut key = [0; 32];
                self.connection
                    .export_keying_material(&mut key, b"miopunch-nat-traversal-v1", &plan.round)
                    .map_err(|_| invalid("enhanced UDP exporter unavailable"))?;
                key
            },
            local: plan.scope.authorization.local,
            remote: plan.scope.authorization.remote,
        };
        trace.step("listeners");
        let mut lease = while_peer_receives(
            &peer,
            self.endpoint
                .register_udp_traversal(&self.connection, plan.round),
        )
        .await?;
        lease.use_primary_binding(plan.scope.binding);
        let check_listener = || {
            if peer.stop_code().is_some() {
                return Err(peer_stopped());
            }
            check()
        };
        let setup = while_peer_receives(
            &peer,
            lease.listen_random_ports_checked(
                usize::from(plan.local_phase.behavior.listen_random),
                &check_listener,
            ),
        )
        .await;
        let listeners = lease.listener_report();
        {
            let mut trace = state.trace.lock().expect("member UDP trace");
            trace.listener_attempted = listeners.attempted;
            trace.listener_bound = listeners.bound;
            trace.listener_failed = listeners.failed;
        }
        state.update(|report| {
            report.listeners = lease
                .local_sockets()
                .into_iter()
                .filter(|socket| !socket.id.is_primary())
                .collect();
        });
        setup?;
        trace.log(
            "listeners-ready",
            format_args!(
                " attempted={} bound={} failed={}",
                listeners.attempted, listeners.bound, listeners.failed
            ),
        );
        exchange_phase(stream, &round.message(wire::LISTENING)).await?;
        trace.step("punch");
        let packets = AtomicUsize::new(super::packet_limit(&plan.local_phase));
        let (winner, mut retained) = while_peer_receives(&peer, async {
            let winner = nat_execution::run(
                &plan.local_phase,
                &mut lease,
                &auth,
                PhaseRun {
                    check: &check,
                    spend: &|| {
                        packets
                            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                                count.checked_sub(1)
                            })
                            .is_ok()
                    },
                    trace: &state.trace,
                    peer_sends_immediately: plan.peer_phase.behavior.send_delay_ms == 0,
                    receiver_wait: false,
                    event: &|event, _| match event {
                        PhaseEvent::FirstSend { .. } => {
                            trace.log("first-send", format_args!(""));
                        }
                        PhaseEvent::FirstMatch { winner, at } => {
                            trace.log(
                                "first-match",
                                format_args!(" socket={}", winner.local_socket_id.get()),
                            );
                            state.update(|report| report.first_match = Some((winner.clone(), at)));
                        }
                    },
                },
            )
            .await?;
            self.permits_winner(&winner)?;
            let retained = lease
                .retain_winner(winner.local_socket_id, winner.local_addr)
                .await?;
            Ok((winner, retained))
        })
        .await?;
        drop(lease);
        state.update(|report| report.winner = Some(winner.clone()));
        // READY confirms both the retained raw socket and its scoped response
        // permission before either peer can start QUIC validation.
        let _mapping = self.connection.retain_nat_mapping(winner.remote, LocalSocketBinding {
            id: winner.local_socket_id,
            local_addr: winner.local_addr,
        });
        trace.step("winner-ready");
        exchange_phase(stream, &round.message(wire::WINNER_READY)).await?;
        slot.release_physical();
        state.update(|report| report.physical_released = Some(StdInstant::now()));
        trace.step("local-path");
        let (path, ack, probe, candidate) =
            while_peer_receives(&peer, self.validate(&winner, trace)).await?;
        state.update(|report| report.validated = Some((path, ack)));
        trace.log("local-ack", format_args!(" path={path:?}"));
        trace.step("peer-ack");
        exchange_phase(stream, &round.message(wire::VALIDATED)).await?;
        // Keep each peer's ACK route available until both validators have finished.
        trace.step("ack-recheck");
        let check_ack = || {
            if probe.is_acknowledged().map_err(io::Error::other)? {
                Ok(())
            } else {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            }
        };
        if kind.automatically_improves_path() {
            let check_backup = || {
                check()?;
                self.permits_winner(&winner)?;
                if let Some(lease) = &retained {
                    lease.check_binding(
                        &self.connection,
                        LocalSocketBinding {
                            id: winner.local_socket_id,
                            local_addr: winner.local_addr,
                        },
                    )?;
                }
                // validate() started this probe on the winner's exact original
                // path. It also detects a closed or rebound path, including on primary UDP.
                check_ack()
            };
            trace.step("prepare-retention");
            check_backup()?;
            // SELECTED is the existing wire value for this barrier. Keep the
            // rollback owners until both validators finish, even on cancellation.
            trace.step("peer-ready");
            exchange_phase(stream, &round.message(wire::SELECTED)).await?;
            stream.finish_send().await?;
            trace.step("retain");
            check_backup()?;
            if let Some(lease) = &mut retained {
                lease.retain();
            }
            if let Some(candidate) = candidate {
                candidate.retain();
            }
            trace.log(
                "retained",
                format_args!(" path={path:?} socket={}", winner.local_socket_id.get()),
            );
            return Ok(());
        }
        check_ack()?;
        drop(probe);
        trace.step("business");
        while_peer_receives(&peer, async {
            while self.on_winner(&winner, |candidate| {
                candidate.id() == path && candidate.is_selected()
            }) != Some(true)
            {
                time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
        .await?;
        let business = while_peer_receives(&peer, async {
            incoming
                .recv()
                .await
                .ok_or(io::ErrorKind::Interrupted.into())
        })
        .await?;
        incoming.close();
        self.finish_business(
            stream,
            plan,
            &winner,
            path,
            retained,
            recommendation,
            state,
            business,
            &peer,
        )
        .await?;
        if let Some(candidate) = candidate {
            candidate.retain();
        }
        Ok(())
    }

    fn permits_winner(&self, winner: &Winner) -> io::Result<()> {
        if self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .is_some_and(|policy| {
                policy.permits_member_data_path(&TransportAddr::Ip(winner.remote))
            })
        {
            Ok(())
        } else {
            Err(denied("member UDP winner no longer permitted"))
        }
    }

    async fn validate(
        &self,
        winner: &Winner,
        trace: &mut RoundTrace,
    ) -> io::Result<(
        PathId,
        noq_proto::PathAcknowledgment,
        noq::PathProbe,
        Option<UdpCandidatePath>,
    )> {
        let local = LocalSocketBinding {
            id: winner.local_socket_id,
            local_addr: winner.local_addr,
        };
        let started = StdInstant::now();
        let candidate =
            if self.connection.side().is_client() && self.on_winner(winner, |_| ()).is_none() {
                loop {
                    self.permits_winner(winner)?;
                    match self
                        .endpoint
                        .inner
                        .open_udp_candidate_from(
                            self.connection.remote_id(),
                            self.connection.stable_id(),
                            local,
                            winner.remote,
                        )
                        .await
                    {
                        Ok(candidate) => break candidate,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            time::sleep(Duration::from_millis(15)).await
                        }
                        Err(error) => return Err(error),
                    }
                }
            } else {
                None
            };
        let (id, probe) = loop {
            if let Some(result) = self.on_winner(winner, |path| {
                path.start_return_path_probe()
                    .map(|probe| (path.id(), probe))
            }) {
                break result?;
            }
            time::sleep(Duration::from_millis(10)).await;
        };
        trace.step("return-probe");
        loop {
            self.permits_winner(winner)?;
            if probe.is_acknowledged().map_err(io::Error::other)?
                && let Some(ack) = self
                    .on_winner(winner, |path| {
                        (path.id() == id)
                            .then(|| path.stats().acknowledgment)
                            .flatten()
                    })
                    .flatten()
                    .filter(|ack| ack.sent_at >= started)
            {
                return Ok((id, ack, probe, candidate));
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn on_winner<T>(&self, winner: &Winner, use_path: impl FnOnce(Path<'_>) -> T) -> Option<T> {
        self.connection
            .paths()
            .iter()
            .find(|path| {
                path.remote_addr() == &TransportAddr::Ip(winner.remote)
                    && path.local_socket_id() == Some(winner.local_socket_id)
                    && path.local_addr() == &LocalTransportAddr::Ip(Some(winner.local_addr.ip()))
            })
            .map(use_path)
    }

    pub(in super::super) async fn arm_business(
        &self,
        round: [u8; 16],
        stream: &RecvStream,
    ) -> Option<(noq::StreamReceivePath, oneshot::Sender<()>)> {
        let execution = self
            .traversal
            .lock()
            .expect("member UDP execution")
            .clone()?;
        let report = execution.report();
        if report.round != round || report.outcome.is_some() {
            return None;
        }
        let (path, _) = report.validated?;
        let observation = self
            .on_winner(&report.winner?, |candidate| {
                (candidate.id() == path && candidate.is_selected())
                    .then(|| candidate.observe_stream_receive(stream))
                    .flatten()
            })
            .flatten()?;
        let (ready, waiting) = oneshot::channel();
        let (confirm, confirmed) = oneshot::channel();
        execution
            .business
            .try_send(Business {
                stream: stream.id(),
                ready,
                confirmed,
            })
            .ok()?;
        waiting.await.ok()?;
        Some((observation, confirm))
    }

    async fn finish_business(
        &self,
        stream: &ControlExchange,
        plan: &MemberUdpPlan,
        winner: &Winner,
        path: PathId,
        mut retained: Option<UdpSocketPathLease>,
        recommendation: Option<Recommendation>,
        state: &Execution,
        business: Business,
        peer: &noq::StreamControl,
    ) -> io::Result<()> {
        let round = Round(plan.round);
        let selected = || {
            self.check_scope(&plan.scope)?;
            if state
                .report
                .lock()
                .expect("member UDP report")
                .physical_released
                .is_none()
            {
                plan.evidence.check()?;
            }
            super::remaining(plan.deadline)?;
            self.permits_winner(winner)?;
            if self.on_winner(winner, |candidate| {
                candidate.id() == path && candidate.is_selected()
            }) != Some(true)
            {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "enhanced UDP winner is no longer selected",
                ));
            }
            Ok(())
        };
        selected()?;
        exchange_phase(
            stream,
            &round.business(wire::BUSINESS_READY, business.stream),
        )
        .await?;
        business
            .ready
            .send(())
            .map_err(|_| io::ErrorKind::Interrupted)?;
        while_peer_receives(peer, async {
            business
                .confirmed
                .await
                .map_err(|_| io::ErrorKind::Interrupted.into())
        })
        .await?;
        selected()?;
        exchange_phase(
            stream,
            &round.business(wire::BUSINESS_CONFIRMED, business.stream),
        )
        .await?;
        stream.finish_send().await?;
        selected()?;
        if let Some(lease) = &mut retained {
            lease.commit(&self.connection)?;
        }
        let completed = StdInstant::now();
        let learned = recommendation.as_ref().is_some_and(|recommendation| {
            self.owner
                .analyzer
                .lock()
                .expect("member UDP analyzer")
                .report_success(recommendation, completed)
        });
        state.update(|report| {
            report.business = Some((business.stream, completed));
            report.learned = learned;
        });
        Ok(())
    }
}

async fn while_peer_receives<T>(
    peer: &noq::StreamControl,
    work: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    // Dropping an unfinished exchange sends STOP_SENDING. Observing it leaves
    // the reader free for the next phase and ignores acknowledgement of a FIN.
    let stopped = async {
        match peer.stopped().await? {
            Some(_) => Err(peer_stopped()),
            None => pending().await,
        }
    };
    tokio::select! { biased;
        result = stopped => result,
        result = work => result,
    }
}

fn peer_stopped() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "peer stopped the UDP coordination stream",
    )
}

async fn exchange_phase(stream: &ControlExchange, message: &[u8]) -> io::Result<()> {
    let (_, incoming) = tokio::try_join!(send(stream, message), receive(stream))?;
    if incoming == message {
        Ok(())
    } else {
        Err(invalid("unexpected enhanced UDP round phase"))
    }
}
