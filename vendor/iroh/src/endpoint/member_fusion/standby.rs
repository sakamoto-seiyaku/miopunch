//! Endpoint-owned retention for admitted, idle connections.
use super::{Activity, ActivityState, Arc, Duration, Mutex, Weak};

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

impl Activity {
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

    pub(super) fn finish_check(self: &Arc<Self>) {
        self.state.lock().expect("member activity lock").checks -= 1;
        self.offer_standby();
    }

    pub(super) fn finish_handoff(self: &Arc<Self>) {
        self.state.lock().expect("member activity lock").handoff = false;
        self.offer_standby();
    }
}
