//! Credential-copy custody is independent of network roles. Offers and saved
//! receipts contain public proofs; private credential bytes travel only during
//! the explicitly confirmed delivery over an authenticated member connection.
use std::io;

use p2panda_core::{Hash, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::management_auth::{ManagementAuth, NetworkAnchor};

pub const CUSTODY_ALPN: &[u8] = b"miopunch/credential-custody/1";
pub const MAX_CUSTODY_OFFERS: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustodyDescription {
    pub network: NetworkAnchor,
    pub source: VerifyingKey,
    pub source_authorization: Hash,
    pub recipient: VerifyingKey,
    pub recipient_authorization: Hash,
    pub nonce: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustodyOffer {
    pub id: Hash,
    pub description: CustodyDescription,
    pub signature: Vec<u8>,
}
impl CustodyOffer {
    pub fn new(
        network: NetworkAnchor,
        auth: &ManagementAuth,
        source: VerifyingKey,
        recipient: VerifyingKey,
        root: &SigningKey,
    ) -> io::Result<Self> {
        if root.verifying_key() != network.root || source == recipient {
            return Err(rejected());
        }
        let description = CustodyDescription {
            network,
            source,
            source_authorization: auth
                .authorization_origin_at(source, &auth.tips(), false)
                .ok_or_else(rejected)?,
            recipient,
            recipient_authorization: auth
                .authorization_origin_at(recipient, &auth.tips(), false)
                .ok_or_else(rejected)?,
            nonce: rand::random(),
        };
        let statement = statement(b"offer", &description)?;
        let offer = Self {
            id: Hash::digest(&statement),
            description,
            signature: root.sign(&statement).to_bytes().to_vec(),
        };
        offer.check_current(network, auth)?;
        Ok(offer)
    }
    pub fn verify(&self, network: NetworkAnchor) -> io::Result<()> {
        let bytes = statement(b"offer", &self.description)?;
        if self.description.network != network
            || self.description.source == self.description.recipient
            || self.id != Hash::digest(&bytes)
            || !verify(network.root, &bytes, &self.signature)
        {
            return Err(rejected());
        }
        Ok(())
    }
    pub fn check_current(&self, network: NetworkAnchor, auth: &ManagementAuth) -> io::Result<()> {
        self.verify(network)?;
        let d = &self.description;
        if auth.authorization_origin_at(d.source, &auth.tips(), false)
            != Some(d.source_authorization)
            || auth.authorization_origin_at(d.recipient, &auth.tips(), false)
                != Some(d.recipient_authorization)
        {
            return Err(rejected());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustodyReceipt {
    pub offer: Hash,
    pub root_signature: Vec<u8>,
    pub recipient_signature: Vec<u8>,
}
impl CustodyReceipt {
    /// Adapters call this only after durable save and independent readback of
    /// the complete root plus canonical Create, never merely after receipt.
    pub fn saved(
        offer: &CustodyOffer,
        root: &SigningKey,
        recipient: &SigningKey,
    ) -> io::Result<Self> {
        if root.verifying_key() != offer.description.network.root
            || recipient.verifying_key() != offer.description.recipient
        {
            return Err(rejected());
        }
        let bytes = statement(b"saved", offer)?;
        Ok(Self {
            offer: offer.id,
            root_signature: root.sign(&bytes).to_bytes().to_vec(),
            recipient_signature: recipient.sign(&bytes).to_bytes().to_vec(),
        })
    }
    pub fn verify(&self, offer: &CustodyOffer) -> io::Result<()> {
        let bytes = statement(b"saved", offer)?;
        if self.offer != offer.id
            || !verify(offer.description.network.root, &bytes, &self.root_signature)
            || !verify(
                offer.description.recipient,
                &bytes,
                &self.recipient_signature,
            )
        {
            return Err(rejected());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustodyEntry {
    pub offer: CustodyOffer,
    pub accepted: bool,
    pub receipt: Option<CustodyReceipt>,
    /// Local recipient decision; retained across reopen and node backups.
    #[serde(default)]
    pub stopped: bool,
}
impl CustodyEntry {
    pub fn accept(&mut self) -> io::Result<()> {
        if self.stopped {
            return Err(rejected());
        }
        self.accepted = true;
        Ok(())
    }

    /// Completed offers are historical evidence, never another save request.
    pub fn delivery(&self) -> io::Result<Option<&CustodyReceipt>> {
        if let Some(receipt) = &self.receipt {
            return Ok(Some(receipt));
        }
        if !self.accepted || self.stopped {
            return Err(rejected());
        }
        Ok(None)
    }

    pub fn recipient_confirmed(&self) -> bool {
        self.accepted && (!self.stopped || self.receipt.is_some())
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CustodyJournal {
    pub entries: Vec<CustodyEntry>,
}
impl CustodyJournal {
    /// Stopping custody cancels every outstanding incoming consent. A new
    /// offer can be accepted independently; old saved receipts remain evidence.
    pub fn stop_received(&mut self, recipient: VerifyingKey) {
        for entry in &mut self.entries {
            if entry.offer.description.recipient == recipient {
                entry.stopped = true;
            }
        }
    }
    pub fn verify(&self, network: NetworkAnchor) -> io::Result<()> {
        if self.entries.len() > MAX_CUSTODY_OFFERS {
            return Err(rejected());
        }
        let mut ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.offer.verify(network)?;
            if !ids.insert(entry.offer.id) {
                return Err(rejected());
            }
            if let Some(receipt) = &entry.receipt {
                if !entry.accepted {
                    return Err(rejected());
                }
                receipt.verify(&entry.offer)?;
            }
        }
        Ok(())
    }
    pub fn get(&self, id: Hash) -> io::Result<&CustodyEntry> {
        self.entries
            .iter()
            .find(|e| e.offer.id == id)
            .ok_or_else(rejected)
    }
    pub fn get_mut(&mut self, id: Hash) -> io::Result<&mut CustodyEntry> {
        self.entries
            .iter_mut()
            .find(|e| e.offer.id == id)
            .ok_or_else(rejected)
    }
    pub fn retain(&mut self, offer: CustodyOffer) -> io::Result<()> {
        if let Ok(existing) = self.get(offer.id) {
            return if existing.offer == offer {
                Ok(())
            } else {
                Err(rejected())
            };
        }
        if self.entries.len() >= MAX_CUSTODY_OFFERS {
            return Err(rejected());
        }
        self.entries.push(CustodyEntry {
            offer,
            accepted: false,
            receipt: None,
            stopped: false,
        });
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustodyProbe {
    pub network: NetworkAnchor,
    pub member: VerifyingKey,
    pub member_authorization: Hash,
    pub challenge: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustodyProof {
    pub probe: CustodyProbe,
    pub signature: Vec<u8>,
}
impl CustodyProof {
    pub fn sign(probe: CustodyProbe, root: &SigningKey) -> io::Result<Self> {
        if root.verifying_key() != probe.network.root {
            return Err(rejected());
        }
        Ok(Self {
            signature: root
                .sign(&statement(b"possession", &probe)?)
                .to_bytes()
                .to_vec(),
            probe,
        })
    }
    pub fn verify(&self, probe: &CustodyProbe) -> io::Result<()> {
        if serde_json::to_vec(&self.probe)? != serde_json::to_vec(probe)?
            || !verify(
                probe.network.root,
                &statement(b"possession", probe)?,
                &self.signature,
            )
        {
            return Err(rejected());
        }
        Ok(())
    }
}

fn statement(domain: &[u8], value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = b"miopunch/credential-custody/1\0".to_vec();
    bytes.extend_from_slice(domain);
    bytes.push(0);
    bytes.extend(serde_json::to_vec(value)?);
    Ok(bytes)
}
fn verify(author: VerifyingKey, statement: &[u8], signature: &[u8]) -> bool {
    let Ok(bytes) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    author.verify(statement, &Signature::from_bytes(&bytes))
}
fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "credential custody request is invalid or no longer current",
    )
}
