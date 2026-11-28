//! Candidate selection and retry state for ordinary UDP on one member connection.

use super::{
    BTreeSet, FxHashMap, HOLEPUNCH_ATTEMPTS_INTERVAL, HashSet, HolepunchAttempt, Instant,
    LOCAL_PROBE_GRACE, MemberPathPolicy, SocketAddr, transports,
};

#[derive(Debug, Default)]
struct FamilyProbes {
    last_exchange: Option<HolepunchAttempt>,
    cursor: usize,
    network_refresh: bool,
    refresh_peers: BTreeSet<SocketAddr>,
}

#[derive(Debug, Default)]
pub(super) struct MemberUdpProbes {
    families: [FamilyProbes; 2],
    rotated_at: Option<Instant>,
    known_probes: FxHashMap<SocketAddr, Instant>,
    // At most one extra target for the current period of concurrent business.
    additional_direct: Option<SocketAddr>,
}

pub(super) struct DirectPaths {
    pub(super) ready: usize,
    pub(super) good_udp: bool,
    pub(super) healthy_tcp: bool,
    pub(super) verified: BTreeSet<SocketAddr>,
}

/// Inputs held constant while the actor applies one reevaluation.
pub(super) struct ProbeConditions<'a> {
    pub(super) direct: &'a DirectPaths,
    pub(super) refresh: [bool; 2],
    pub(super) paused: [bool; 2],
    pub(super) now: Instant,
}

impl ProbeConditions<'_> {
    pub(super) fn can_probe(&self, addr: &SocketAddr) -> bool {
        (!self.direct.healthy_tcp || self.refresh[usize::from(addr.is_ipv6())])
            && !self.paused[usize::from(addr.ip().to_canonical().is_ipv6())]
    }
}

impl MemberUdpProbes {
    pub(super) fn peer_added(&mut self, address: SocketAddr) {
        let family = &mut self.families[usize::from(address.is_ipv6())];
        family.network_refresh = true;
        family.refresh_peers.insert(address);
    }

    pub(super) fn network_changed(&mut self, changed: [bool; 2], valid: &HashSet<SocketAddr>) {
        self.known_probes
            .retain(|addr, _| valid.contains(addr) || !changed[usize::from(addr.is_ipv6())]);
        if self
            .additional_direct
            .is_some_and(|addr| changed[usize::from(addr.is_ipv6())])
        {
            self.additional_direct = None;
        }
        for (family, changed) in self.families.iter_mut().zip(changed) {
            if changed {
                family.last_exchange = None;
                family.network_refresh = true;
            }
        }
    }

    pub(super) fn refresh_needed(
        &mut self,
        verified: &BTreeSet<SocketAddr>,
        local: &BTreeSet<SocketAddr>,
    ) -> [bool; 2] {
        std::array::from_fn(|index| {
            let family = &mut self.families[index];
            let in_family = |addr: &SocketAddr| addr.is_ipv6() == (index == 1);
            if verified.iter().any(|addr| {
                in_family(addr)
                    && (family.refresh_peers.is_empty() || family.refresh_peers.contains(addr))
            }) {
                family.network_refresh = false;
                family.refresh_peers.clear();
            }
            family.network_refresh && local.iter().any(in_family)
        })
    }

    pub(super) fn clear_additional_direct(&mut self) {
        self.additional_direct = None;
    }

    #[cfg(not(wasm_browser))]
    pub(super) fn waiting_for_candidates(&self, now: Instant) -> bool {
        self.families
            .iter()
            .filter_map(|f| f.last_exchange.as_ref())
            .any(|last| {
                last.remote_candidates.is_empty() && now - last.when < HOLEPUNCH_ATTEMPTS_INTERVAL
            })
    }

    pub(super) fn local_window(
        &mut self,
        all: &BTreeSet<SocketAddr>,
        limit: usize,
        update: &ProbeConditions<'_>,
    ) -> BTreeSet<SocketAddr> {
        if all.len() > limit
            && update.direct.ready == 0
            && self
                .rotated_at
                .is_none_or(|last| update.now - last >= HOLEPUNCH_ATTEMPTS_INTERVAL)
        {
            if self.rotated_at.is_some() {
                for family in &mut self.families {
                    family.cursor = family.cursor.wrapping_add((limit / 2).max(1));
                }
            }
            self.rotated_at = Some(update.now);
        }
        member_candidate_window(all, limit, self.families.each_ref().map(|f| f.cursor))
    }

    pub(super) fn known_targets(
        &mut self,
        known: &BTreeSet<SocketAddr>,
        explicit: &BTreeSet<SocketAddr>,
        balancing: bool,
        update: &ProbeConditions<'_>,
    ) -> Vec<SocketAddr> {
        self.known_probes.retain(|remote, _| known.contains(remote));
        let additional = if balancing {
            self.additional_direct.or_else(|| {
                explicit
                    .iter()
                    .chain(known)
                    .copied()
                    .find(|addr| update.can_probe(addr))
            })
        } else {
            None
        };
        let may_retry = update.refresh.contains(&true)
            || !update.direct.good_udp
            || (balancing && self.additional_direct.is_none());
        known
            .iter()
            .copied()
            .filter(|addr| {
                let wanted = if balancing {
                    additional == Some(*addr)
                } else {
                    explicit.contains(addr)
                        || update.refresh[usize::from(addr.is_ipv6())]
                        || update.paused.contains(&true)
                };
                wanted
                    && update.can_probe(addr)
                    && self.known_probes.get(addr).is_none_or(|last| {
                        may_retry && update.now - *last >= HOLEPUNCH_ATTEMPTS_INTERVAL
                    })
            })
            .collect()
    }

    pub(super) fn probe_started(&mut self, remote: SocketAddr, now: Instant, balancing: bool) {
        self.known_probes.insert(remote, now);
        if balancing {
            self.additional_direct = Some(remote);
        }
    }

    fn recently_probed(
        &self,
        ipv6: bool,
        now: Instant,
        eligible: impl Fn(&SocketAddr) -> bool,
    ) -> bool {
        self.known_probes.iter().any(|(remote, sent)| {
            remote.is_ipv6() == ipv6 && now - *sent < LOCAL_PROBE_GRACE && eligible(remote)
        })
    }

    pub(super) fn enhanced_fallback(&self, needed: [bool; 2], now: Instant) -> [bool; 2] {
        std::array::from_fn(|index| {
            let recent_exchange = self.families[index]
                .last_exchange
                .as_ref()
                .is_some_and(|last| now - last.when < LOCAL_PROBE_GRACE);
            needed[index] && !recent_exchange && !self.recently_probed(index == 1, now, |_| true)
        })
    }

    pub(super) fn server_advertisements(
        &self,
        local: BTreeSet<SocketAddr>,
        announced: &[SocketAddr],
        update: &ProbeConditions<'_>,
    ) -> BTreeSet<SocketAddr> {
        let verified = &update.direct.verified;
        let have_ipv6 = verified
            .iter()
            .any(|addr| addr.ip().to_canonical().is_ipv6());
        local
            .into_iter()
            .filter(|addr| {
                announced.contains(addr)
                    || (!self.recently_probed(addr.is_ipv6(), update.now, |_| true)
                        && update.can_probe(addr)
                        && (verified.is_empty()
                            || update.refresh[usize::from(addr.is_ipv6())]
                            || (addr.ip().to_canonical().is_ipv6() && !have_ipv6)))
            })
            .collect()
    }

    pub(super) fn next_exchange(
        &self,
        ipv6: bool,
        local: &BTreeSet<SocketAddr>,
        advertised: &[SocketAddr],
        known: &BTreeSet<SocketAddr>,
        policy: &MemberPathPolicy,
        update: &ProbeConditions<'_>,
    ) -> Option<HolepunchAttempt> {
        let family = usize::from(ipv6);
        if update.paused[family] {
            return None;
        }
        let local_candidates: BTreeSet<_> = local
            .iter()
            .copied()
            .filter(|addr| addr.is_ipv6() == ipv6 && update.can_probe(addr))
            .collect();
        if local_candidates.is_empty() {
            return None;
        }
        let remote_candidates: BTreeSet<_> = advertised
            .iter()
            .copied()
            .filter(|addr| addr.is_ipv6() == ipv6 && policy.permits(transports::Addr::Ip(*addr)))
            .collect();
        let verified = &update.direct.verified;
        if self.recently_probed(ipv6, update.now, |addr| update.can_probe(addr))
            || (!verified.is_empty()
                && remote_candidates.is_subset(verified)
                && !known.iter().any(|addr| addr.is_ipv6() == ipv6))
        {
            return None;
        }
        let last = self.families[family].last_exchange.as_ref();
        if last.is_some_and(|last| {
            local_candidates.is_subset(&last.local_candidates)
                && remote_candidates.is_subset(&last.remote_candidates)
                && (update.direct.good_udp || update.now - last.when < HOLEPUNCH_ATTEMPTS_INTERVAL)
        }) {
            return None;
        }
        Some(HolepunchAttempt {
            when: update.now,
            local_candidates,
            remote_candidates,
        })
    }

    pub(super) fn exchange_started(&mut self, ipv6: bool, attempt: HolepunchAttempt) {
        self.families[usize::from(ipv6)].last_exchange = Some(attempt);
    }
}

/// Give both families space in QNT's negotiated window, rotating excess candidates on retry.
fn member_candidate_window(
    candidates: &BTreeSet<SocketAddr>,
    limit: usize,
    cursors: [usize; 2],
) -> BTreeSet<SocketAddr> {
    if candidates.len() <= limit {
        return candidates.clone();
    }
    let v4: Vec<_> = candidates
        .iter()
        .copied()
        .filter(SocketAddr::is_ipv4)
        .collect();
    let v6: Vec<_> = candidates
        .iter()
        .copied()
        .filter(SocketAddr::is_ipv6)
        .collect();
    let mut ordered = Vec::with_capacity(candidates.len());
    for index in 0..v4.len().max(v6.len()) {
        for (family, cursor) in [&v4, &v6].into_iter().zip(cursors) {
            if index < family.len() {
                ordered.push(family[(cursor % family.len() + index) % family.len()]);
            }
        }
    }
    ordered.into_iter().take(limit).collect()
}
