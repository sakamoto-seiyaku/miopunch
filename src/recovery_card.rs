//! Portable genesis credential backup. This contains no node identity, history,
//! or admission secret. Treat both the plain card and its password as secrets.
use std::{io, num::NonZeroU32};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p2panda_core::{Hash, SigningKey};
use ring::{aead, pbkdf2};

use crate::management_auth::NetworkAnchor;

const PLAIN: &str = "miopunch-recovery:1:";
const ENCRYPTED: &str = "miopunch-recovery-encrypted:1:";
const AAD: &[u8] = b"miopunch/genesis-recovery-card/1";
const ITERATIONS: NonZeroU32 = NonZeroU32::new(600_000).unwrap();

/// Complete original credential: independent Ed25519 seed and canonical Create
/// pin. No Debug or Serialize implementation: export must be an explicit action.
pub struct RecoveryCard {
    seed: [u8; 32],
    genesis: Hash,
}
impl RecoveryCard {
    pub fn new(seed: [u8; 32], genesis: Hash) -> Self {
        Self { seed, genesis }
    }
    pub fn anchor(&self) -> NetworkAnchor {
        NetworkAnchor {
            root: SigningKey::from_bytes(&self.seed).verifying_key(),
            genesis: self.genesis,
        }
    }
    /// For a credential recovery adapter, never diagnostics or invitation data.
    pub fn credential(&self) -> ([u8; 32], Hash) {
        (self.seed, self.genesis)
    }
    /// None exports the printable/password-manager form. A supplied password
    /// opts into PBKDF2-HMAC-SHA256 + AES-256-GCM with fresh salt and nonce.
    pub fn export(&self, password: Option<&str>) -> io::Result<String> {
        let mut bytes = self.seed.to_vec();
        bytes.extend_from_slice(self.genesis.as_bytes());
        let Some(password) = password else {
            return Ok(format!("{PLAIN}{}", URL_SAFE_NO_PAD.encode(bytes)));
        };
        let salt: [u8; 16] = rand::random();
        let nonce: [u8; 12] = rand::random();
        let key = encryption_key(password, &salt)?;
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut bytes,
        )
        .map_err(|_| invalid())?;
        let mut envelope = salt.to_vec();
        envelope.extend_from_slice(&nonce);
        envelope.extend(bytes);
        Ok(format!("{ENCRYPTED}{}", URL_SAFE_NO_PAD.encode(envelope)))
    }
    pub fn import(text: &str, password: Option<&str>) -> io::Result<Self> {
        let text = text.trim();
        if text.len() > 256 {
            return Err(invalid());
        }
        let bytes = if let Some(value) = text.strip_prefix(PLAIN) {
            URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?
        } else if let Some(value) = text.strip_prefix(ENCRYPTED) {
            let mut bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
            if bytes.len() != 16 + 12 + 64 + aead::AES_256_GCM.tag_len() {
                return Err(invalid());
            }
            let key = encryption_key(password.ok_or_else(invalid)?, &bytes[..16])?;
            let nonce = bytes[16..28].try_into().map_err(|_| invalid())?;
            key.open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(AAD),
                &mut bytes[28..],
            )
            .map_err(|_| invalid())?
            .to_vec()
        } else {
            return Err(invalid());
        };
        if bytes.len() != 64 {
            return Err(invalid());
        }
        Ok(Self::new(
            bytes[..32].try_into().unwrap(),
            Hash::from_bytes(bytes[32..].try_into().unwrap()),
        ))
    }
}
fn encryption_key(password: &str, salt: &[u8]) -> io::Result<aead::LessSafeKey> {
    if password.is_empty() || password.len() > 1024 {
        return Err(invalid());
    }
    let mut key = [0; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        ITERATIONS,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| invalid())?,
    ))
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "recovery card rejected")
}
