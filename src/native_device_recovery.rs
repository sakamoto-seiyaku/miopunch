//! Recover original-network management using the existing root-proof protocol.
//! Preparing a new identity is distinct from restoring a complete node backup.
use std::{net::Ipv4Addr, path::Path};

use iroh::EndpointAddr;
use serde_json::{Value, json};

use crate::{
    control_protocol::AuthenticatedControl,
    genesis_recovery::GenesisCredential,
    management_recovery::MANAGEMENT_RECOVERY_ALPN,
    native_client::{ClientError, NativeClient},
    native_network::NativeNetwork,
    recovery_card::RecoveryCard,
    resources::ResourceConfig,
};

impl NativeClient {
    /// Explicit confirmation of recovery onto this new independent node.
    /// The card identifies the original network; a current ordinary member can
    /// supply history and admission material without old-administrator approval.
    pub async fn prepare_management_recovery(
        path: impl AsRef<Path>,
        card: &RecoveryCard,
        contact: iroh_base::EndpointId,
        bind: Ipv4Addr,
        configuration: ResourceConfig,
    ) -> Result<Self, ClientError> {
        configuration.validate_contact_facilities()?;
        let (seed, genesis) = card.credential();
        let credential = GenesisCredential::from_saved(seed, genesis);
        let node = NativeNetwork::recover(path, &credential, contact).await?;
        node.save_resource_configuration(&configuration)?;
        super::native_client::record_received_credential(&node)?;
        Self::start(node, bind).await
    }

    /// Retries retain the durable recovery attempt and exact signed grant.
    pub async fn restore_management_authority(
        &self,
        contact: EndpointAddr,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let connection = self
            .resources
            .connect_recovery_control(contact, MANAGEMENT_RECOVERY_ALPN)
            .await?;
        let result = {
            let mut node = self.node.lock().await;
            let result = node.restore_management(&connection).await;
            self.files
                .update_access(self.services.lock().await.access(&node).await?);
            result.map(|progress| {
                json!({"stage":format!("{:?}", progress.stage),
                "authorization":progress.authorization,"node":node.status().node_id,
                "network":node.anchor,"saved_locally":true,
                "old_administrators_automatically_revoked":false,
                "scope":"this original network; new node identity, no business data recovery"})
            })
        };
        connection.close(0u32.into(), b"management recovery operation ended");
        Ok(result?)
    }

    pub async fn management_recovery_status(&self) -> Result<Value, ClientError> {
        let node = self.node.lock().await;
        let progress = node.recovery_progress().await?;
        Ok(
            json!({"stage":format!("{:?}",progress.stage),"authorization":progress.authorization,
            "node":node.status().node_id,"network":node.anchor}),
        )
    }
}
