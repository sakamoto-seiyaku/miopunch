//! Saved MID1 startup permission and SDK ownership. The carrier is installed
//! before Endpoint bind, then attached only to an actual admitted outer M.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct MemberDataConfiguration {
    pub(super) via: String,
    #[serde(default)]
    pub(super) forward_data: bool,
    #[serde(default)]
    pub(super) disabled: bool,
}
impl MemberDataConfiguration {
    pub(super) fn validate(&self) -> Result<(), JsValue> {
        self.via.parse::<iroh::EndpointId>().map_err(error)?;
        Ok(())
    }
}

pub(super) struct MemberDataQualificationSource {
    pub(super) original: iroh::endpoint::WeakConnectionHandle,
    permitted: Arc<AtomicBool>,
    updates: n0_watcher::Watchable<u64>,
}
impl MemberDataQualificationSource {
    pub(super) fn retire(&mut self) {
        if self.permitted.swap(false, Ordering::AcqRel) {
            let _ = self.updates.set(1);
        }
    }
}
impl Drop for MemberDataQualificationSource {
    fn drop(&mut self) {
        self.retire();
    }
}

impl BrowserGate {
    pub(super) fn member_data_allowed(saved: &SavedNode, via: iroh::EndpointId) -> bool {
        saved
            .resources
            .as_ref()
            .and_then(|resources| resources.member_data.as_ref())
            .is_some_and(|entry| {
                entry.forward_data
                    && !entry.disabled
                    && entry.via.parse::<iroh::EndpointId>().ok() == Some(via)
            })
    }
    pub(super) fn retire_member_data(&self) {
        let mut gate = self.0.lock().unwrap();
        for source in gate.member_data_sources.values_mut() {
            source.retire();
        }
    }
    async fn member_data_permission(
        &self,
        original: &Connection,
    ) -> Result<crate::member_data::MemberDataPermission, JsValue> {
        // Async store validation precedes token minting; no JS bool is an
        // admission or resource grant. The callback below is lock-free.
        self.qualified_snapshot(original).await?;
        let mut gate = self.0.lock().unwrap();
        let saved = gate.saved.as_ref().ok_or_else(rejected)?;
        if original.alpn() != BUSINESS_ALPN
            || !Self::proved_member_current(&gate, saved, original)
            || !Self::member_data_allowed(saved, original.remote_id())
            || original.paths().iter().any(|path| matches!(path.remote_addr(),
                iroh::TransportAddr::Custom(address) if address.id() == crate::member_data::DATA_TRANSPORT_ID))
        { return Err(error("MID1 requires proved outer M and saved explicit ForwardData permission")); }
        gate.member_data_sources
            .retain(|_, source| source.original.is_open());
        if gate.member_data_sources.len() >= 16 {
            return Err(error("too many owned member data leases"));
        }
        if gate.member_data_sources.contains_key(&original.stable_id()) {
            return Err(error("outer member already owns a MID1 generation"));
        }
        let source = MemberDataQualificationSource {
            original: original.weak_handle(),
            permitted: Arc::new(AtomicBool::new(true)),
            updates: n0_watcher::Watchable::new(0),
        };
        let permitted = source.permitted.clone();
        let permission = crate::member_data::MemberDataPermission::qualified(
            original,
            source.updates.watch(),
            move || permitted.load(Ordering::Acquire),
        )
        .map_err(error)?;
        gate.member_data_sources
            .insert(original.stable_id(), source);
        Ok(permission)
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Attach this node's startup-installed data client to an admitted outer
    /// member M from this same SDK node. Expiry never closes that outer M.
    pub async fn attach_member_data(
        &mut self,
        outer: &BrowserMemberConnection,
        lifetime_ms: u32,
        caller_ms: u32,
    ) -> Result<String, JsValue> {
        if self.member_data_lease.is_some() {
            return Err(error("MID1 already attached"));
        }
        if lifetime_ms == 0
            || lifetime_ms > 300000
            || caller_ms == 0
            || Duration::from_millis(caller_ms.into()) > self.caller_budget
            || self.endpoint.id() != outer.endpoint.id()
            || !Arc::ptr_eq(&self.gate.0, &outer.gate.0)
        {
            return Err(error("invalid MID1 attach scope or budget"));
        }
        let transport = self
            .member_data_transport
            .as_ref()
            .ok_or_else(|| error("MID1 not installed by saved startup configuration"))?
            .clone();
        let attempt = crate::connection_coordinator::AttemptContext::new(
            n0_future::time::Instant::now() + Duration::from_millis(caller_ms.into()),
            tokio_util::sync::CancellationToken::new(),
        );
        let permission = n0_future::time::timeout(
            attempt
                .deadline
                .saturating_duration_since(n0_future::time::Instant::now()),
            async {
                outer.qualified_snapshot().await?;
                self.gate.member_data_permission(&outer.connection).await
            },
        )
        .await
        .map_err(|_| error("MID1 attach caller deadline during qualification"))??;
        let lease = transport
            .attach(
                &outer.connection,
                permission,
                Duration::from_millis(lifetime_ms.into()),
                &attempt,
            )
            .await
            .map_err(error)?;
        self.member_data_lease = Some(lease);
        self.member_data_address()
    }
    /// Existing 48-byte MID1 address: forwarding node identity + fresh route.
    /// Exchange this dynamic hint through an authorized contact mechanism.
    pub fn member_data_address(&self) -> Result<String, JsValue> {
        let address = self
            .member_data_transport
            .as_ref()
            .ok_or_else(rejected)?
            .address()
            .map_err(error)?;
        Ok(serde_json::json!({"transportId":address.id(),"dynamicRoute":hex::encode(address.data())}).to_string())
    }
    pub fn member_data_diagnostics(&self) -> String {
        let mut report = self
            .member_data_transport
            .as_ref()
            .map(|transport| transport.report())
            .unwrap_or_else(
                || serde_json::json!({"installed":false,"activeTasks":0,"ready":false}),
            );
        report["savedPathSelectionPolicy"] = serde_json::json!(
            self.saved
                .resources
                .as_ref()
                .map(|resources| &resources.path_selection)
        );
        report.to_string()
    }
    /// Normal member/1 coordinator dial, using the actual remote MID1 token.
    /// The hint does not skip target membership proof or release checks.
    pub fn member_data_connection_attempt(
        &self,
        target: String,
        dynamic_route: String,
        caller_ms: u32,
    ) -> Result<BrowserMemberAttempt, JsValue> {
        if caller_ms == 0 || Duration::from_millis(caller_ms.into()) > self.caller_budget {
            return Err(error(
                "member caller budget must fit the original browser budget",
            ));
        }
        let target: iroh::EndpointId = target.parse().map_err(error)?;
        if !BrowserGate::permits(&self.saved, target, BUSINESS_ALPN) {
            return Err(rejected());
        }
        if dynamic_route.len() != 96 {
            return Err(error("invalid MID1 route length"));
        }
        let bytes = hex::decode(dynamic_route).map_err(error)?;
        let route =
            iroh_base::CustomAddr::from_parts(crate::member_data::DATA_TRANSPORT_ID, &bytes);
        if !self
            .member_data_transport
            .as_ref()
            .is_some_and(|transport| transport.permits_address(&route))
        {
            return Err(error(
                "MID1 route does not match the current forwarding member",
            ));
        }
        Ok(BrowserMemberAttempt {
            endpoint: self.endpoint.clone(),
            gate: self.gate.clone(),
            lease: self.lease.as_ref().ok_or_else(rejected)?.0,
            saved: self.committed.clone(),
            address: EndpointAddr::new(target).with_addrs([iroh::TransportAddr::Custom(route)]),
            deadline: n0_future::time::Instant::now() + Duration::from_millis(caller_ms.into()),
            cancel: tokio_util::sync::CancellationToken::new(),
            started: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "rtc-probe")]
            rtc_transport: self.rtc_transport.clone(),
        })
    }
    /// Disable is durable and terminal for this generation; only MID1 retires.
    /// A later explicit enabled startup creates a fresh token, never revives it.
    pub async fn disable_member_data(&mut self) -> Result<String, JsValue> {
        let mut saved = self.saved.clone();
        let entry = saved
            .resources
            .as_mut()
            .and_then(|resources| resources.member_data.as_mut())
            .ok_or_else(rejected)?;
        entry.disabled = true;
        let next = serde_json::to_string(&saved).map_err(error)?;
        save_node(
            self.lease.as_ref().ok_or_else(rejected)?.0,
            JsValue::from_str(&self.committed),
            next.clone(),
        )
        .await?;
        self.saved = saved;
        self.committed = next;
        self.gate.install(&self.saved);
        if let Some(mut lease) = self.member_data_lease.take() {
            let _ = lease.close().await;
        }
        Ok(self.member_data_diagnostics())
    }
}
