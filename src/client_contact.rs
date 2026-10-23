//! Native adaptation of a trusted invitation's portable contact descriptors.
use crate::{
    client_invitation::{BootstrapContact, ClientInvitation},
    native_network::NativeNetwork,
    resources::{Resource, ResourceConfig, ResourceUse},
};
use std::{collections::BTreeSet, io};

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
        &node.directory.join("client.invitation"),
        &invitation.encode()?,
    )
}
