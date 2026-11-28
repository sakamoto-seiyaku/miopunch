//! A browser node can publish several snapshots without changing its signed
//! history. Share the verification of those exact immutable records.
use std::{
    ops::Deref,
    sync::{Arc, OnceLock},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::management_auth::{AuthError, ManagementAuth, NetworkAnchor};
use crate::management_record::stored_records;
use crate::service_protocol::ManagementRecords;

#[derive(Clone, Default)]
struct History {
    records: ManagementRecords,
    verified: OnceLock<(NetworkAnchor, Arc<ManagementAuth>)>,
}

#[derive(Clone, Default)]
pub(super) struct AppliedHistory(Arc<History>);

impl From<ManagementRecords> for AppliedHistory {
    fn from(records: ManagementRecords) -> Self {
        Self(Arc::new(History {
            records,
            verified: OnceLock::new(),
        }))
    }
}

impl Deref for AppliedHistory {
    type Target = ManagementRecords;

    fn deref(&self) -> &Self::Target {
        &self.0.records
    }
}

impl AppliedHistory {
    pub(super) fn verified(&self, anchor: NetworkAnchor) -> Result<&ManagementAuth, AuthError> {
        let verified = &self.0.verified;
        let view = match verified.get() {
            Some(view) => view,
            None => {
                let auth = ManagementAuth::replay_records(anchor, self)?;
                verified.get_or_init(|| (anchor, Arc::new(auth)))
            }
        };
        if view.0 != anchor {
            return Err(AuthError::InvalidEnvelope);
        }
        Ok(&view.1)
    }

    pub(super) fn push(&mut self, record: (Vec<u8>, Option<Vec<u8>>)) {
        let history = Arc::make_mut(&mut self.0);
        history.verified.take();
        history.records.push(record);
    }

    /// The merge applied each appended record to this view before publishing it.
    pub(super) fn remember_verified(&self, anchor: NetworkAnchor, auth: ManagementAuth) {
        self.0.verified.get_or_init(|| (anchor, Arc::new(auth)));
    }
}

impl Serialize for AppliedHistory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        stored_records::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for AppliedHistory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(stored_records::deserialize(deserializer)?.into())
    }
}
