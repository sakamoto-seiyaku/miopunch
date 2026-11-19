//! The node's one enhanced UDP slot. A protected offer may yield before it does work.

use std::{
    io,
    sync::{Arc, Mutex},
};

use super::{CancellationToken, Duration, Instant, busy, remaining};
use crate::{endpoint::ObservationValidity, socket::transports::QadSubmission};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    Maintenance,
    Business,
}

#[derive(Debug, Default)]
pub(super) struct Slot(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    active: Option<Active>,
    cooldown: Option<(ObservationValidity, Instant)>,
    business: usize,
}

#[derive(Debug)]
struct Active {
    holder: Arc<Holder>,
    committed: bool,
}

#[derive(Debug)]
struct Holder {
    local: [u8; 32],
    peer: [u8; 32],
    connection: usize,
    outgoing: bool,
    priority: Priority,
    reserved: Instant,
    yielded: CancellationToken,
}

impl Holder {
    fn spend(&self, state: &mut State, generation: &ObservationValidity, interval: Duration) {
        let until = self.reserved + interval;
        if state.cooldown.as_ref().is_none_or(|(_, due)| *due != until) {
            tracing::debug!(stable_id = self.connection, ?until, "member UDP slot spent");
            state.cooldown = Some((generation.clone(), until));
        }
    }
}

pub(super) struct Reservation<'a> {
    slot: &'a Slot,
    holder: Arc<Holder>,
}

impl Slot {
    pub(super) fn business_changed(&self, active: bool) {
        let mut state = self.0.lock().expect("member UDP slot");
        if active {
            state.business += 1;
            if state.active.as_ref().is_some_and(|active| {
                !active.committed && active.holder.priority == Priority::Maintenance
            }) {
                Self::yield_active(&mut state);
            }
        } else {
            state.business -= 1;
        }
    }

    fn yield_active(state: &mut State) {
        let old = state.active.take().expect("active UDP reservation");
        tracing::debug!(
            stable_id = old.holder.connection,
            peer = ?old.holder.peer,
            outgoing = old.holder.outgoing,
            "member UDP slot yielded before work"
        );
        old.holder.yielded.cancel();
    }

    pub(super) fn reserve(
        &self,
        scope: &super::LearningScope,
        connection: usize,
        outgoing: bool,
        priority: Priority,
        deadline: Instant,
        traverse: bool,
    ) -> io::Result<Reservation<'_>> {
        let mut state = self.0.lock().expect("member UDP slot");
        remaining(deadline)?;
        let now = Instant::now();
        if traverse
            && let Some((generation, until)) = &state.cooldown
            && generation.is_current()
            && now < *until
        {
            tracing::debug!(stable_id = connection, due_in = ?(*until - now), "member UDP slot cooling down");
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "member UDP round cooling down",
            ));
        }
        if priority == Priority::Maintenance && state.business != 0 {
            return Err(busy());
        }
        if let Some(active) = &state.active {
            let holder = &active.holder;
            let crossed = !outgoing && holder.outgoing && holder.peer == scope.remote;
            if crossed {
                tracing::debug!(
                    stable_id = connection,
                    occupied_by = holder.connection,
                    peer = ?scope.remote,
                    committed = active.committed,
                    "member UDP crossed offers"
                );
            }
            let wins = priority > holder.priority
                || (priority == holder.priority && crossed && scope.remote < holder.local);
            if active.committed || !wins {
                return Err(busy());
            }
            Self::yield_active(&mut state);
        }
        let holder = Arc::new(Holder {
            local: scope.local,
            peer: scope.remote,
            connection,
            outgoing,
            priority,
            reserved: now,
            yielded: CancellationToken::new(),
        });
        tracing::debug!(stable_id = connection, peer = ?scope.remote, outgoing, ?priority, "member UDP slot reserved");
        state.active = Some(Active {
            holder: holder.clone(),
            committed: false,
        });
        Ok(Reservation { slot: self, holder })
    }
}

impl Reservation<'_> {
    pub(super) fn yielded(&self) -> &CancellationToken {
        &self.holder.yielded
    }

    pub(super) fn commit(&self) -> io::Result<()> {
        let mut state = self.slot.0.lock().expect("member UDP slot");
        let active = state
            .active
            .as_mut()
            .filter(|active| Arc::ptr_eq(&active.holder, &self.holder))
            .ok_or_else(busy)?;
        active.committed = true;
        Ok(())
    }

    pub(super) fn spend(&self, generation: &ObservationValidity, interval: Duration) {
        let mut state = self.slot.0.lock().expect("member UDP slot");
        self.holder.spend(&mut state, generation, interval);
    }

    pub(super) fn qad_submission(
        &self,
        generation: ObservationValidity,
        interval: Duration,
    ) -> QadSubmission {
        let slot = self.slot.0.clone();
        let holder = self.holder.clone();
        QadSubmission::new(move || {
            let mut state = slot.lock().expect("member UDP slot");
            if state
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(&active.holder, &holder))
            {
                holder.spend(&mut state, &generation, interval);
            }
        })
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut state = self.slot.0.lock().expect("member UDP slot");
        if state
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(&active.holder, &self.holder))
        {
            state.active = None;
        }
    }
}
