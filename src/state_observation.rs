use tokio::sync::watch;

/// A runtime-scoped cursor. A fresh starting point also makes a cursor from
/// a previously closed instance immediately eligible for a new snapshot.
#[derive(Clone)]
pub(crate) struct StateChanges(watch::Sender<u64>);

impl Default for StateChanges {
    fn default() -> Self {
        Self(watch::Sender::new(rand::random()))
    }
}

impl StateChanges {
    pub(crate) fn publish(&self) {
        self.0
            .send_modify(|cursor| *cursor = cursor.wrapping_add(1));
    }

    pub(crate) fn cursor(&self) -> u64 {
        *self.0.borrow()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}
