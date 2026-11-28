//! The browser node's strict IndexedDB commit and runtime publication.
use super::*;
use std::rc::Rc;

#[wasm_bindgen(module = "/src/storage.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = saveNode)]
    async fn save_with_receipt(
        token: u32,
        previous: JsValue,
        next: String,
        receipt: &str,
    ) -> Result<JsValue, JsValue>;
}

/// An optimistic edit stays bound to the disk version and lease it read.
pub(super) struct NodeVersion<'a> {
    gate: &'a BrowserGate,
    previous: String,
    pub(super) lease: u32,
}

/// Keep the writer through a recheck and the associated material release.
pub(super) struct NodeWrite<'a> {
    version: NodeVersion<'a>,
    _writer: tokio::sync::MutexGuard<'a, ()>,
}

impl BrowserGate {
    async fn read_version(&self) -> Result<(Box<SavedNode>, NodeVersion<'_>), JsValue> {
        let (saved, token, migration_owner) = {
            let state = self.0.lock().unwrap();
            (
                Box::new(state.saved.as_ref().ok_or_else(rejected)?.clone()),
                state.lease.ok_or_else(rejected)?,
                state.migration_owner.clone(),
            )
        };
        let raw = serde_json::to_string(&saved).map_err(error)?;
        let expected = if let Some(owner) = migration_owner {
            let owner = owner.upgrade().ok_or_else(rejected)?;
            let state = owner.lock().unwrap();
            let active = state.saved.as_ref().ok_or_else(rejected)?;
            let inactive = &active.migration.as_ref().ok_or_else(rejected)?.inactive;
            if serde_json::to_string(inactive).map_err(error)? != raw {
                return Err(error("browser migration context changed"));
            }
            serde_json::to_string(active).map_err(error)?
        } else {
            raw.clone()
        };
        if read_node(token).await?.as_string().as_deref() != Some(&expected) {
            return Err(error("browser service state changed or storage cleared"));
        }
        Ok((
            saved,
            NodeVersion {
                gate: self,
                previous: raw,
                lease: token,
            },
        ))
    }

    pub(super) async fn snapshot(&self) -> Result<(SavedNode, NodeVersion<'_>), JsValue> {
        let _writer = self.2.lock().await;
        let (saved, version) = self.read_version().await?;
        Ok((*saved, version))
    }

    pub(super) async fn installed(&self) -> Result<Box<SavedNode>, JsValue> {
        // Admission polls this future inside the WASM handshake. Keep the full
        // node out of the nested protocol frames.
        let _writer = self.2.lock().await;
        Ok(self.read_version().await?.0)
    }

    pub(super) async fn write_node(&self) -> Result<(SavedNode, NodeWrite<'_>), JsValue> {
        let guard = self.2.lock().await;
        let (saved, version) = self.read_version().await?;
        Ok((
            *saved,
            NodeWrite {
                version,
                _writer: guard,
            },
        ))
    }

    pub(super) async fn update_saved<T>(
        &self,
        edit: impl FnOnce(&mut SavedNode) -> Result<T, JsValue>,
    ) -> Result<T, JsValue> {
        let (mut saved, mut write) = self.write_node().await?;
        let result = edit(&mut saved)?;
        let raw = encode_saved(&saved)?;
        if raw != write.version.previous {
            write.version.persist(&saved, raw, None, None).await?;
        }
        Ok(result)
    }
}

impl NodeVersion<'_> {
    // An IndexedDB abort must leave all runtime observers on the old state.
    async fn persist(
        &mut self,
        next: &SavedNode,
        raw: String,
        receipt: Option<&str>,
        retire: Option<&Rc<files::BrowserFiles>>,
    ) -> Result<(), JsValue> {
        let previous = JsValue::from_str(&self.previous);
        if let Some(receipt) = receipt {
            save_with_receipt(self.lease, previous, raw.clone(), receipt).await?;
        } else {
            save_node(self.lease, previous, raw.clone()).await?;
        }
        if let Some(files) = retire {
            files.reset_for_network();
        }
        self.gate.install(next);
        self.previous = raw;
        Ok(())
    }

    pub(super) async fn commit(mut self, next: &SavedNode) -> Result<(), JsValue> {
        let _guard = self.gate.2.lock().await;
        self.persist(next, encode_saved(next)?, None, None).await
    }
}

impl NodeWrite<'_> {
    pub(super) async fn commit(&mut self, next: &SavedNode) -> Result<(), JsValue> {
        self.version
            .persist(next, encode_saved(next)?, None, None)
            .await
    }

    pub(super) async fn switch(
        &mut self,
        next: &SavedNode,
        receipt: Option<&str>,
        files: &Rc<files::BrowserFiles>,
    ) -> Result<(), JsValue> {
        self.version
            .persist(next, encode_saved(next)?, receipt, Some(files))
            .await
    }
}

impl BrowserNetwork {
    pub(super) async fn refresh_service_snapshot(&mut self) -> Result<(), JsValue> {
        self.lease.as_ref().ok_or_else(rejected)?;
        let (saved, version) = self.gate.snapshot().await?;
        self.saved = saved;
        self.committed = version.previous;
        Ok(())
    }

    pub(super) async fn commit(&mut self, next: SavedNode) -> Result<(), JsValue> {
        self.commit_node(next, None).await
    }

    pub(super) async fn commit_node(
        &mut self,
        next: SavedNode,
        receipt: Option<&str>,
    ) -> Result<(), JsValue> {
        let lease = self.lease.as_ref().ok_or_else(rejected)?.0;
        let _guard = self.gate.2.lock().await;
        let mut version = NodeVersion {
            gate: &self.gate,
            lease,
            previous: self.committed.clone(),
        };
        version
            .persist(&next, encode_saved(&next)?, receipt, None)
            .await?;
        self.committed = version.previous;
        self.saved = next;
        Ok(())
    }
}

fn encode_saved(next: &SavedNode) -> Result<String, JsValue> {
    next.verify()?;
    next.validate_migration()?;
    let raw = serde_json::to_string(next).map_err(error)?;
    if raw.len() > 80 * 1024 * 1024 {
        return Err(error("browser state too large"));
    }
    Ok(raw)
}
