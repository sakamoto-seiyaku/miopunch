//! Self-authored display metadata. Membership and authorization use device keys.
use std::{collections::BTreeMap, io};

use p2panda_auth::Access;
use p2panda_core::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

const MAX_NAME_BYTES: usize = 256;
const DOMAIN: &[u8] = b"miopunch/device-name/1\0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DeviceName {
    pub(crate) device: VerifyingKey,
    revision: u64,
    name: String,
    signature: Signature,
}

impl DeviceName {
    pub(crate) fn valid(&self) -> bool {
        self.revision > 0
            && valid_name(&self.name)
            && self
                .device
                .verify(&statement(self.revision, &self.name), &self.signature)
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

fn statement(revision: u64, name: &str) -> Vec<u8> {
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(&revision.to_be_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

pub(crate) fn with_display_name(
    mut value: serde_json::Value,
    name: Option<&str>,
) -> serde_json::Value {
    if let Some(name) = name {
        value["display_name"] = serde_json::json!(name);
    }
    value
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DeviceNames(BTreeMap<VerifyingKey, DeviceName>);

impl DeviceNames {
    pub(crate) fn name(&self, id: VerifyingKey) -> Option<&str> {
        self.record(id).map(|record| record.name.as_str())
    }

    pub(crate) fn record(&self, id: VerifyingKey) -> Option<&DeviceName> {
        self.0.get(&id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn display_names(
        &self,
        ids: impl Iterator<Item = VerifyingKey>,
    ) -> BTreeMap<VerifyingKey, String> {
        ids.filter_map(|id| self.name(id).map(|name| (id, name.to_owned())))
            .collect()
    }

    pub(crate) fn set_local(&mut self, key: &SigningKey, name: &str) -> io::Result<()> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid device display name",
            ));
        }
        let device = key.verifying_key();
        if self.name(device) == Some(name) {
            return Ok(());
        }
        let revision = self
            .record(device)
            .map_or(0, |record| record.revision)
            .checked_add(1)
            .ok_or_else(|| io::Error::other("device display name revision exhausted"))?;
        let signature = key.sign(&statement(revision, name));
        self.0.insert(
            device,
            DeviceName {
                device,
                revision,
                name: name.into(),
                signature,
            },
        );
        Ok(())
    }

    /// A host-provided default is a one-time seed; a persisted choice wins.
    pub(crate) fn seed_local(&mut self, key: &SigningKey, name: &str) {
        if self.record(key.verifying_key()).is_none() && valid_name(name.trim()) {
            self.set_local(key, name).expect("valid first device name");
        }
    }

    /// Install a record whose device signature has already been verified.
    pub(crate) fn remember(&mut self, name: DeviceName) {
        let current = self.record(name.device);
        if current.is_none_or(|old| (name.revision, &name.name) > (old.revision, &old.name)) {
            self.0.insert(name.device, name);
        }
    }

    pub(crate) fn for_members(&self, members: &[(VerifyingKey, Access)]) -> Self {
        Self(
            members
                .iter()
                .filter_map(|(id, _)| self.record(*id).map(|name| (*id, name.clone())))
                .collect(),
        )
    }

    pub(crate) fn merge_members(&mut self, members: &[(VerifyingKey, Access)], mut names: Self) {
        for (id, _) in members {
            if let Some(name) = names.0.remove(id)
                && name.device == *id
                && name.valid()
            {
                self.remember(name);
            }
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.0
            .iter()
            .all(|(id, name)| *id == name.device && name.valid())
    }
}
