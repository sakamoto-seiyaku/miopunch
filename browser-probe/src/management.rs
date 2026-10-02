//! Foreground, device-local management session. Root material is never saved.
use super::*;
use crate::management_action::ManagementAction;
use crate::management_auth::ManagementExtension;
use n0_future::time::Instant;
use p2panda_auth::GroupsExtensionArgs;
use p2panda_core::{Header, SigningKey};

pub(super) struct Session {
    id: String,
    elevation: Option<(SigningKey, Instant)>,
}
impl Session {
    fn elevated(&mut self) -> bool {
        if self
            .elevation
            .as_ref()
            .is_some_and(|(_, deadline)| *deadline <= Instant::now())
        {
            self.elevation = None;
        }
        self.elevation.is_some()
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn begin_management_session(&mut self) -> Result<String, JsValue> {
        if self.lease.is_none() {
            return Err(rejected());
        }
        let id = hex::encode(rand::random::<[u8; 16]>());
        self.management = Some(Session {
            id: id.clone(),
            elevation: None,
        });
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
        let key: [u8; 32] = hex::decode(seed)
            .map_err(error)?
            .try_into()
            .map_err(|_| rejected())?;
        let key = SigningKey::from_bytes(&key);
        let anchor = self.saved.invitation()?.anchor;
        if key.verifying_key() != anchor.root
            || genesis != anchor.genesis.to_string()
            || window_ms == 0
        {
            return Err(rejected());
        }
        let session = self.session(&session)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(window_ms.into()))
            .ok_or_else(rejected)?;
        session.elevation = Some((key, deadline));
        Ok(())
    }
    pub fn exit_genesis_elevation(&mut self, session: String) -> Result<(), JsValue> {
        self.session(&session)?.elevation = None;
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
        self.check_store().await?;
        let target: VerifyingKey = target.parse().map_err(error)?;
        let action = match action.as_str() {
            "add-member" => ManagementAction::AddMember(target),
            "remove-member" => ManagementAction::RemoveMember(target),
            "appoint-administrator" => ManagementAction::AppointAdministrator(target),
            "demote-administrator" => ManagementAction::DemoteAdministrator(target),
            _ => return Err(error("unknown management action")),
        };
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        let current = auth
            .nodes()
            .into_iter()
            .find(|(id, _)| *id == target)
            .map(|(_, access)| access);
        let elevated = self.session(&session)?.elevated();
        if action.requires_genesis(current) && !elevated {
            return Err(error("genesis elevation required"));
        }
        let (author, deadline) = if elevated {
            let elevation = self.session(&session)?.elevation.as_ref().unwrap();
            (elevation.0.clone(), Some(elevation.1))
        } else {
            (SigningKey::from_bytes(&self.saved.key), None)
        };
        let header = self.author_action(&auth, &author, action, deadline)?;
        let record = self.ingest_record(header, None).await?;
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
    pub(super) fn author_action(
        &self,
        auth: &ManagementAuth,
        author: &SigningKey,
        action: ManagementAction,
        deadline: Option<Instant>,
    ) -> Result<Vec<u8>, JsValue> {
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
                group_id: self.saved.invitation()?.anchor.root,
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
