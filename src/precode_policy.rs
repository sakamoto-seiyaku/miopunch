//! Platform-independent rules for a *new* designated-node redemption.
//! Existing results instead revalidate their original membership episode.
#[derive(Clone, Copy)]
pub(crate) struct RedemptionPolicy {
    pub expires: u64,
    pub max_uses: Option<u64>,
    pub used: u64,
    pub stopped: bool,
    pub revoked: bool,
    pub ledger_consistent: bool,
    pub issuer_authorized: bool,
}
impl RedemptionPolicy {
    pub fn permits_new(self, now: u64) -> bool {
        valid_limits(self.expires, self.max_uses, now)
            && !self.stopped
            && !self.revoked
            && self.ledger_consistent
            && self.issuer_authorized
            && self.used < i64::MAX as u64
            && self.max_uses.is_none_or(|limit| self.used < limit)
    }
}
pub(crate) fn valid_limits(expires: u64, max_uses: Option<u64>, now: u64) -> bool {
    expires > now
        && expires <= i64::MAX as u64
        && max_uses.is_none_or(|limit| limit > 0 && limit <= i64::MAX as u64)
}
