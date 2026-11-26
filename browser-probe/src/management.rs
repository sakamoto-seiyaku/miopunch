//! Foreground, device-local management session. Elevation does not persist its
//! supplied root; explicit creator custody is separate from this legacy API.
use super::*;
use crate::management_action::ManagementAction;
use crate::management_auth::ManagementExtension;
use n0_future::time::Instant;
use p2panda_auth::GroupsExtensionArgs;
use p2panda_core::{Header, SigningKey};

pub(super) struct Session {
    id: String,
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn begin_management_session(&mut self) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        if self.lease.is_none() || saved.creator_seed.is_some() && !saved.recovery_saved {
            return Err(rejected());
        }
        let id = hex::encode(rand::random::<[u8; 16]>());
        self.management = Some(Session { id: id.clone() });
        Ok(id)
    }
    /// Explicitly supplied root seed and pinned canonical Create. The caller
    /// chooses the temporary window. Neither seed nor elevation enters IDB.
    pub fn elevate_genesis(
        &mut self,
        session: String,
        seed: String,
        genesis: String,
        window_ms: u32,
    ) -> Result<(), JsValue> {
        self.session(&session)?;
        let _ = (seed, genesis, window_ms);
        Err(error(
            "general genesis windows are retired; propose and confirm one device operation",
        ))
    }
    pub fn exit_genesis_elevation(&mut self, session: String) -> Result<(), JsValue> {
        self.session(&session)?;
        Ok(())
    }
    pub fn end_management_session(&mut self, session: String) -> Result<(), JsValue> {
        self.session(&session)?;
        self.management = None;
        Ok(())
    }
    pub async fn manage(
        &mut self,
        session: String,
        action: String,
        target: String,
    ) -> Result<String, JsValue> {
        let target: VerifyingKey = target.parse().map_err(error)?;
        let action = match action.as_str() {
            "add-member" => ManagementAction::AddMember(target),
            "remove-member" => ManagementAction::RemoveMember(target),
            "appoint-administrator" => ManagementAction::AppointAdministrator(target),
            "demote-administrator" => ManagementAction::DemoteAdministrator(target),
            _ => return Err(error("unknown management action")),
        };
        self.session(&session)?;
        self.require_device_ready()?;
        let record = services::update_saved(&self.gate, |saved| {
            let auth = saved.verify()?.ok_or_else(rejected)?;
            let current = auth
                .nodes()
                .into_iter()
                .find(|(id, _)| *id == target)
                .map(|(_, access)| access);
            if action.requires_genesis(current) {
                return Err(error("this action requires its own genesis confirmation"));
            }
            let header =
                saved.author_action(&auth, &SigningKey::from_bytes(&saved.key), action, None)?;
            let id = Hash::digest(&header);
            *saved = saved.merge_records(vec![(header, None)])?;
            Ok(id)
        })
        .await?;
        Ok(record.to_string())
    }
    pub async fn leave(&mut self, session: String) -> Result<String, JsValue> {
        self.manage(
            session,
            "remove-member".into(),
            self.saved.identity().to_string(),
        )
        .await
    }
}
impl BrowserNetwork {
    fn session(&mut self, id: &str) -> Result<&mut Session, JsValue> {
        self.management
            .as_mut()
            .filter(|session| session.id == id)
            .ok_or_else(|| error("management session ended or replaced"))
    }
}

impl SavedNode {
    pub(super) fn author_action(
        &self,
        auth: &ManagementAuth,
        author: &SigningKey,
        action: ManagementAction,
        deadline: Option<Instant>,
    ) -> Result<Vec<u8>, JsValue> {
        if matches!(action, ManagementAction::AddMember(_)) {
            self.admissions_open()?;
        }
        let target = action.target();
        let current = auth
            .nodes()
            .into_iter()
            .find(|(id, _)| *id == target)
            .map(|(_, access)| access);
        let log_id = loop {
            let id = rand::random();
            if !auth.has_author_log_id(author.verifying_key(), id) {
                break id;
            }
        };
        let extension = ManagementExtension {
            version: 1,
            log_id,
            group: GroupsExtensionArgs {
                group_id: self.invitation()?.anchor.root,
                action: action.group_action(current),
                dependencies: auth.tips(),
            },
        };
        // Graph/log preparation can consume the last part of the window.
        // As on native, do not sign with an expired root credential.
        if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
            return Err(error("genesis elevation required"));
        }
        let header = Header::<ManagementExtension>::builder()
            .build(author, extension)
            .encode();
        let mut check = auth.clone();
        check.accept(&header).map_err(error)?;
        Ok(header)
    }
}
