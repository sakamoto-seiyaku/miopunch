//! Native adaptation of a trusted invitation's portable contact descriptors.
use crate::{
    client_invitation::{BootstrapContact, ClientInvitation},
    native_network::NativeNetwork,
    resources::{Resource, ResourceConfig, ResourceUse},
};
use std::{collections::BTreeSet, io};

const SAVED_INVITATION: &str = "client.invitation";
type RejoinInvitations = ([u8; 16], Option<ClientInvitation>, ClientInvitation);

pub(crate) fn import_contact(
    mut local: ResourceConfig,
    contact: &BootstrapContact,
) -> io::Result<ResourceConfig> {
    contact.validate()?;
    // An explicit replacement is a user's allow-list, including an empty one.
    // Invitations can add network facilities to defaults, never undo that list
    // or any disabled origin/use.
    if local.replacement.is_none() {
        let resources = contact
            .relays
            .iter()
            .map(|url| (url, ResourceUse::DataRelay))
            .chain(
                contact
                    .mqtt
                    .iter()
                    .map(|f| (&f.native_url, ResourceUse::Signaling)),
            )
            .chain(
                contact
                    .irc
                    .iter()
                    .map(|f| (&f.native_url, ResourceUse::Signaling)),
            )
            .chain(
                contact
                    .native_signaling
                    .iter()
                    .map(|url| (url, ResourceUse::Signaling)),
            );
        for (url, usage) in resources {
            if let Some(resource) = local.defaults.resources.iter_mut().find(|r| r.url == *url) {
                resource.uses.insert(usage);
            } else {
                local.defaults.resources.push(Resource {
                    url: url.clone(),
                    uses: BTreeSet::from([usage]),
                    limitation: "trusted complete-invitation contact; not membership authority"
                        .into(),
                });
            }
        }
    }
    // Keep trusted mapping metadata even when local policy does not permit
    // the facility. Its native origin must remain reserved against raw
    // fallback, including default-port aliases in an explicit replacement.
    // Retaining metadata grants no use: allowed_*_facilities still applies
    // the selected resource list and all local denials before binding/export.
    for facility in &contact.mqtt {
        if let Some(old) = local
            .mqtt_facilities
            .iter()
            .find(|old| old.logical_id == facility.logical_id)
        {
            if old != facility {
                return Err(io::Error::other("invitation changes a saved MQTT mapping"));
            }
        } else {
            local.mqtt_facilities.push(facility.clone());
        }
    }
    for facility in &contact.irc {
        if let Some(old) = local
            .irc_facilities
            .iter()
            .find(|old| old.logical_id == facility.logical_id)
        {
            if old != facility {
                return Err(io::Error::other("invitation changes a saved IRC mapping"));
            }
        } else {
            local.irc_facilities.push(facility.clone());
        }
    }
    local.validate_contact_facilities()?;
    Ok(local)
}
pub(crate) fn save_invitation(
    node: &NativeNetwork,
    invitation: &ClientInvitation,
) -> io::Result<()> {
    crate::native_files::write_private_atomic(
        &node.directory.join(SAVED_INVITATION),
        &invitation.encode()?,
    )
}

pub(crate) fn load_invitation(node: &NativeNetwork) -> io::Result<Option<ClientInvitation>> {
    let bytes = match std::fs::read(node.directory.join(SAVED_INVITATION)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !bytes.starts_with(b"[") {
        return ClientInvitation::decode(&bytes).map(Some);
    }
    if let Ok((request, previous, next)) = serde_json::from_slice::<RejoinInvitations>(&bytes) {
        let invitation = if node.join.is_some_and(|binding| binding.request_id == request) {
            Some(next)
        } else {
            previous
        };
        if let Some(invitation) = &invitation {
            invitation.encode()?;
        }
        return Ok(invitation);
    }
    let [previous, next]: [ClientInvitation; 2] = serde_json::from_slice(&bytes)?;
    let binding = node.join.ok_or_else(|| io::Error::other("no saved request binding"))?;
    for invitation in [previous, next] {
        invitation.encode()?;
        if invitation.material_bytes()? == binding.invitation.encode() {
            return Ok(Some(invitation));
        }
    }
    Err(io::Error::other("saved invitations do not match the local request"))
}

/// A new code can have the same contact and network as the prior invitation.
/// The durable request ID selects the full invitation on either side of publish.
pub(crate) fn stage_rejoin_invitation(
    node: &NativeNetwork,
    next: &ClientInvitation,
    request_id: [u8; 16],
) -> io::Result<()> {
    next.encode()?;
    let bytes = serde_json::to_vec(&(request_id, load_invitation(node)?, next))?;
    crate::native_files::write_private_atomic(&node.directory.join(SAVED_INVITATION), &bytes)
}

/// Stage a replacement contact before changing the durable request binding.
/// The current binding selects its invitation on either side of an interruption.
pub(crate) fn stage_invitation(node: &NativeNetwork, next: &ClientInvitation) -> io::Result<()> {
    if let (Some(previous), Some(binding)) = (load_invitation(node)?, node.join) {
        let previous_material = previous.material_bytes()?;
        if previous_material == binding.invitation.encode()
            && previous_material != next.material_bytes()?
        {
            next.encode()?;
            let bytes = serde_json::to_vec(&[&previous, next])?;
            return crate::native_files::write_private_atomic(
                &node.directory.join(SAVED_INVITATION),
                &bytes,
            );
        }
    }
    save_invitation(node, next)
}
