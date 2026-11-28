//! Stream activity and idle retention on a shared member connection.

use super::{
    Arc, Connection, Duration, Instant, Mutex, SendStream, StreamKind, Weak, io, time, udp, watch,
};

/// Lifetime policy for a shared member connection without a business stream.
#[derive(Clone, Copy, Debug)]
pub enum MemberRetention {
    /// Release the connection after this much idle time.
    IdleTimeout(Duration),
    /// Keep up to this many idle peers at this endpoint.
    Standby(usize),
}

#[derive(Default, Debug)]
pub(super) struct StandbyPool(Mutex<Vec<Weak<Activity>>>);

#[derive(Debug)]
pub(in crate::endpoint) struct Activity {
    state: Mutex<ActivityState>,
    business: watch::Sender<bool>,
    concurrent_business: watch::Sender<bool>,
    planner: Arc<udp::Planner>,
    standby: Option<(Arc<StandbyPool>, usize, Connection)>,
    path_activity: watch::Sender<bool>,
}

#[derive(Debug)]
struct ActivityState {
    idle_since: Option<Instant>,
    holders: usize,
    checks: usize,
    used: bool,
    handoff: bool,
    ended: bool,
}

impl Activity {
    pub(super) fn new(
        planner: Arc<udp::Planner>,
        standby: Option<(Arc<StandbyPool>, usize, Connection)>,
    ) -> Self {
        Self {
            state: Mutex::new(ActivityState {
                idle_since: Some(Instant::now()),
                holders: 0,
                checks: 0,
                used: false,
                handoff: true,
                ended: false,
            }),
            business: watch::Sender::default(),
            concurrent_business: watch::Sender::default(),
            planner,
            standby,
            path_activity: watch::Sender::default(),
        }
    }

    pub(in crate::endpoint) fn business_activity(&self) -> watch::Receiver<bool> {
        self.business.subscribe()
    }

    pub(in crate::endpoint) fn concurrent_business(&self) -> watch::Receiver<bool> {
        self.concurrent_business.subscribe()
    }

    pub(in crate::endpoint) fn path_activity(&self) -> watch::Receiver<bool> {
        self.path_activity.subscribe()
    }

    pub(super) fn permits_path_work(&self) -> bool {
        *self.path_activity.borrow()
    }

    pub(super) fn idle_since(&self) -> Option<Instant> {
        self.state.lock().expect("member activity lock").idle_since
    }

    // A standby TCP traversal yields when another session needs the node's budget.
    pub(super) async fn wait_for_foreground_elsewhere(&self) {
        if self.standby.is_none() {
            return std::future::pending().await;
        }
        let mut foreground = self.planner.foreground();
        let mut local = self.business_activity();
        loop {
            let busy = *foreground.borrow_and_update();
            if busy && self.idle_since().is_some() {
                return;
            }
            tokio::select! {
                _ = foreground.changed() => {},
                _ = local.changed() => {},
            }
        }
    }

    fn set_business_holders(&self, holders: usize, state: &mut ActivityState) {
        let previous = std::mem::replace(&mut state.holders, holders);
        let active = holders != 0;
        if (previous != 0) != active {
            if !state.ended {
                state.used |= active;
                state.idle_since = if active { None } else { Some(Instant::now()) };
            }
            self.planner.business_changed(active);
            self.business.send_replace(active);
            // The standby pool owns path eligibility while its sessions are idle.
            if !state.ended && (active || self.standby.is_none()) {
                self.path_activity.send_replace(active);
            }
        }
        if (previous >= 2) != (holders >= 2) {
            self.concurrent_business.send_replace(holders >= 2);
        }
    }

    pub(super) fn end(&self) {
        let mut state = self.state.lock().expect("member activity lock");
        state.ended = true;
        self.set_business_holders(0, &mut state);
    }

    pub(super) fn lease(self: &Arc<Self>, kind: StreamKind) -> io::Result<Option<ActivityLease>> {
        let business = kind == StreamKind::Business;
        if !business && self.standby.is_none() {
            return Ok(None);
        }
        let mut state = self.state.lock().expect("member activity lock");
        if state.ended {
            return Err(io::ErrorKind::NotConnected.into());
        }
        state.handoff = false;
        let kind = if business {
            self.set_business_holders(state.holders + 1, &mut state);
            ActivityLeaseKind::Business(None)
        } else {
            state.checks += 1;
            ActivityLeaseKind::Maintenance(None)
        };
        Ok(Some(ActivityLease {
            activity: self.clone(),
            kind,
        }))
    }

    fn finish_business(self: &Arc<Self>) {
        let mut state = self.state.lock().expect("member activity lock");
        if state.ended {
            return;
        }
        self.set_business_holders(state.holders - 1, &mut state);
        if state.holders == 0 {
            // Pool enrollment takes the pool lock before any activity lock.
            drop(state);
            self.offer_standby();
        }
    }

    pub(super) async fn retain(
        self: &Arc<Self>,
        connection: &Connection,
        retention: MemberRetention,
        handoff_timeout: Duration,
    ) {
        let MemberRetention::IdleTimeout(budget) = retention else {
            // An overflow dial has time to deliver its first stream. Abandoned
            // handoffs must eventually release their transport as well.
            time::sleep(handoff_timeout).await;
            self.finish_handoff();
            return std::future::pending().await;
        };
        let mut changed = self.business_activity();
        loop {
            let idle_since = self.idle_since();
            if let Some(since) = idle_since {
                if time::timeout_at(since + budget, changed.changed())
                    .await
                    .is_err()
                {
                    let state = self.state.lock().expect("member activity lock");
                    if state.idle_since == Some(since) {
                        connection.close(0u32.into(), b"member connection idle");
                        return;
                    }
                }
            } else {
                let _ = changed.changed().await;
            }
        }
    }

    pub(super) fn offer_standby(self: &Arc<Self>) {
        let Some((pool, limit, _)) = &self.standby else {
            return;
        };
        let mut members = pool.0.lock().expect("member standby pool");
        let mut state = self.state.lock().expect("member activity lock");
        if state.ended || state.holders != 0 {
            return;
        }
        let mut enrolled = false;
        members.retain(|entry| {
            let Some(other) = entry.upgrade() else {
                return false;
            };
            if Arc::ptr_eq(self, &other) {
                enrolled = true;
                return true;
            }
            let other = other.state.lock().expect("member activity lock");
            !other.ended && other.holders == 0
        });
        if enrolled {
            return;
        }
        if members.len() == *limit && state.used {
            // Maintenance never updates idle_since. Prefer evicting a peer used
            // only for checks, then the least recently used business peer.
            let oldest = members
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    let other = entry.upgrade()?;
                    let other = other.state.lock().expect("member activity lock");
                    Some(((other.used, other.idle_since), i))
                })
                .min();
            if let Some((rank, index)) = oldest
                && (state.used, state.idle_since) > rank
            {
                if let Some(other) = members.swap_remove(index).upgrade() {
                    let mut victim = other.state.lock().expect("member activity lock");
                    other.release_overflow(&mut victim);
                }
            }
        }
        if members.len() < *limit {
            members.push(Arc::downgrade(self));
            self.path_activity.send_if_modified(|eligible| {
                let changed = !*eligible;
                *eligible = true;
                changed
            });
        } else {
            self.release_overflow(&mut state);
        }
    }

    fn release_overflow(&self, state: &mut ActivityState) {
        if state.holders != 0 || state.ended {
            return;
        }
        self.path_activity.send_if_modified(|eligible| {
            let changed = *eligible;
            *eligible = false;
            changed
        });
        if !state.handoff && state.checks == 0 {
            state.ended = true;
            self.standby
                .as_ref()
                .expect("standby connection")
                .2
                .close(0u32.into(), b"member standby full");
        }
    }

    fn finish_check(self: &Arc<Self>) {
        self.state.lock().expect("member activity lock").checks -= 1;
        self.offer_standby();
    }

    fn finish_handoff(self: &Arc<Self>) {
        self.state.lock().expect("member activity lock").handoff = false;
        self.offer_standby();
    }
}

#[derive(Debug)]
pub(super) struct ActivityLease {
    activity: Arc<Activity>,
    kind: ActivityLeaseKind,
}

#[derive(Debug)]
enum ActivityLeaseKind {
    Business(Option<noq::StreamControl>),
    Maintenance(Option<(tokio::runtime::Handle, noq::StreamControl)>),
}

impl ActivityLease {
    pub(super) fn is_business(&self) -> bool {
        matches!(self.kind, ActivityLeaseKind::Business(_))
    }

    pub(super) fn claim_path(&mut self, send: &SendStream) -> io::Result<()> {
        if let ActivityLeaseKind::Business(control) = &mut self.kind {
            send.mark_business().map_err(io::Error::other)?;
            *control = Some(send.control());
        }
        Ok(())
    }

    pub(super) fn track_maintenance(&mut self, send: &SendStream) {
        if let ActivityLeaseKind::Maintenance(control) = &mut self.kind {
            *control = Some((tokio::runtime::Handle::current(), send.control()));
        }
    }
}

impl Drop for ActivityLease {
    fn drop(&mut self) {
        let activity = &self.activity;
        match &mut self.kind {
            ActivityLeaseKind::Business(control) => {
                if let Some(control) = control.take() {
                    control.release_business();
                }
                activity.finish_business();
            }
            ActivityLeaseKind::Maintenance(control) => {
                if let Some((runtime, control)) = control.take() {
                    let activity = activity.clone();
                    runtime.spawn(async move {
                        let _ = control.stopped().await;
                        activity.finish_check();
                    });
                } else {
                    activity.finish_check();
                }
            }
        }
    }
}
