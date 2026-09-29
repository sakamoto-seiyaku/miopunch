//! A bounded SCRAM-SHA-256-PLUS qualification on one authenticated iroh connection.
//! Successful proof grants only bootstrap eligibility; membership remains a separate decision.

use iroh::endpoint::Connection;
use iroh_base::EndpointId;
use rsasl::callback::{Context, Request, SessionCallback, SessionData};
use rsasl::config::SASLConfig;
use rsasl::mechanisms::scram::{SCRAM_SHA256_PLUS, properties::ScramStoredPassword, tools};
use rsasl::prelude::{
    ChannelBindingCallback, Registry, SASLClient, SASLServer, SessionError, State,
};
use rsasl::property::{AuthId, AuthzId, OverrideCBType, Password};
use rsasl::validate::{Validate, Validation, ValidationError};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

const EXPORTER_LABEL: &[u8] = b"EXPORTER-Channel-Binding";
const BINDING_NAME: &str = "tls-exporter";
const ITERATIONS: u32 = 16_384;
const MAX_FRAME: usize = 512;
const PROOF_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ACTIVE_PROOFS: usize = 32;
const MAX_LIVE_PROOFS: usize = 4096;
static ACTIVE_PROOFS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_ACTIVE_PROOFS);
static ATTEMPTED_CONNECTIONS: LazyLock<Mutex<HashMap<usize, Weak<()>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct FixedBinding([u8; 32]);
impl ChannelBindingCallback for FixedBinding {
    fn get_cb_data(&self, name: &str) -> Option<&[u8]> {
        (name == BINDING_NAME).then_some(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionScope {
    network_id: [u8; 32],
    secret_generation: u64,
}

impl AdmissionScope {
    pub const fn new(network_id: [u8; 32], secret_generation: u64) -> Self {
        Self {
            network_id,
            secret_generation,
        }
    }

    pub const fn network_id(&self) -> &[u8; 32] {
        &self.network_id
    }

    pub const fn secret_generation(&self) -> u64 {
        self.secret_generation
    }

    fn auth_id(&self) -> String {
        format!(
            "miopunch:{}:{}",
            hex::encode(self.network_id),
            self.secret_generation
        )
    }
}

/// A generated, high-entropy network admission secret. Callers must keep saved bytes private.
#[derive(Clone)]
pub struct AdmissionSecret([u8; 32]);

impl std::fmt::Debug for AdmissionSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmissionSecret([redacted])")
    }
}

impl AdmissionSecret {
    pub fn generate() -> Self {
        Self(rand::random())
    }
    pub const fn from_saved(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn saved_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn scram_password(&self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Debug)]
pub struct BootstrapProof {
    connection: Connection,
    scope: AdmissionScope,
    _attempt_token: Arc<()>,
}

impl Drop for BootstrapProof {
    fn drop(&mut self) {
        self.connection.close(0u32.into(), b"admission proof ended");
    }
}

impl BootstrapProof {
    pub fn peer(&self) -> EndpointId {
        self.connection.remote_id()
    }

    pub fn scope(&self) -> &AdmissionScope {
        &self.scope
    }

    /// A proof only applies to its original live connection and credential generation.
    pub fn applies_to(&self, connection: &Connection, scope: &AdmissionScope) -> bool {
        self.scope == *scope
            && self.connection.stable_id() == connection.stable_id()
            && self.connection.close_reason().is_none()
    }

    /// End the proved protocol session, for example after admission-secret rotation.
    pub fn invalidate(&self) {
        self.connection
            .close(0u32.into(), b"admission proof invalidated");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    Timeout,
    Busy,
    AlreadyAttempted,
    Rejected,
    Transport,
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProofError {}

struct ClientCredentials {
    auth_id: String,
    password: String,
}

impl SessionCallback for ClientCredentials {
    fn callback(
        &self,
        _: &SessionData,
        _: &Context,
        request: &mut Request,
    ) -> Result<(), SessionError> {
        request
            .satisfy::<AuthId>(&self.auth_id)?
            .satisfy::<Password>(self.password.as_bytes())?
            .satisfy::<OverrideCBType>(BINDING_NAME)?;
        Ok(())
    }

    fn enable_channel_binding(&self) -> bool {
        true
    }
}

struct ServerCredentials {
    auth_id: String,
    salt: [u8; 32],
    stored_key: [u8; 32],
    server_key: [u8; 32],
}

struct ProofValidation;
impl Validation for ProofValidation {
    type Value = bool;
}

impl ServerCredentials {
    fn new(scope: &AdmissionScope, secret: &AdmissionSecret) -> Self {
        let salt: [u8; 32] = rand::random();
        let mut salted = sha2::digest::Output::<Sha256>::default();
        tools::hash_password::<Sha256>(
            secret.scram_password().as_bytes(),
            ITERATIONS,
            &salt,
            &mut salted,
        );
        let (client_key, server_key) = tools::derive_keys::<Sha256>(&salted);
        Self {
            auth_id: scope.auth_id(),
            salt,
            stored_key: Sha256::digest(client_key).into(),
            server_key: server_key.into(),
        }
    }
}

impl SessionCallback for ServerCredentials {
    fn callback(
        &self,
        _: &SessionData,
        context: &Context,
        request: &mut Request,
    ) -> Result<(), SessionError> {
        if context.get_ref::<AuthId>() == Some(self.auth_id.as_str())
            && context.get_ref::<AuthzId>().is_none()
        {
            request.satisfy::<ScramStoredPassword>(&ScramStoredPassword::new(
                ITERATIONS,
                &self.salt,
                &self.stored_key,
                &self.server_key,
            ))?;
        }
        Ok(())
    }

    fn validate(
        &self,
        _: &SessionData,
        context: &Context,
        result: &mut Validate<'_>,
    ) -> Result<(), ValidationError> {
        result.finalize::<ProofValidation>(
            context.get_ref::<AuthId>() == Some(self.auth_id.as_str())
                && context.get_ref::<AuthzId>().is_none(),
        );
        Ok(())
    }
}

fn config(
    callback: impl SessionCallback + 'static,
) -> Result<std::sync::Arc<SASLConfig>, ProofError> {
    static MECHANISMS: &[rsasl::prelude::Mechanism] = &[SCRAM_SHA256_PLUS];
    SASLConfig::builder()
        .with_registry(Registry::with_mechanisms(MECHANISMS))
        .with_callback(callback)
        .map_err(|_| ProofError::Rejected)
}

fn exporter(connection: &Connection) -> Result<[u8; 32], ProofError> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, EXPORTER_LABEL, b"")
        .map_err(|_| ProofError::Transport)?;
    Ok(binding)
}

struct AttemptGuard {
    connection: Connection,
    token: Option<Arc<()>>,
}

impl AttemptGuard {
    fn succeed(mut self, scope: &AdmissionScope) -> Result<BootstrapProof, ProofError> {
        if self.connection.close_reason().is_some() {
            return Err(ProofError::Transport);
        }
        Ok(BootstrapProof {
            connection: self.connection.clone(),
            scope: scope.clone(),
            _attempt_token: self.token.take().expect("attempt token present"),
        })
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        if self.token.is_some() {
            self.connection
                .close(0u32.into(), b"admission proof failed");
        }
    }
}

fn reserve_once(connection: &Connection) -> Result<AttemptGuard, ProofError> {
    if connection.close_reason().is_some() {
        return Err(ProofError::AlreadyAttempted);
    }
    let id = connection.stable_id();
    let mut attempts = ATTEMPTED_CONNECTIONS
        .lock()
        .map_err(|_| ProofError::Rejected)?;
    attempts.retain(|_, token| token.strong_count() > 0);
    if attempts.contains_key(&id) {
        return Err(ProofError::AlreadyAttempted);
    }
    if attempts.len() >= MAX_LIVE_PROOFS {
        return Err(ProofError::Busy);
    }
    let token = Arc::new(());
    attempts.insert(id, Arc::downgrade(&token));
    Ok(AttemptGuard {
        connection: connection.clone(),
        token: Some(token),
    })
}

async fn send_frame(send: &mut iroh::endpoint::SendStream, bytes: &[u8]) -> Result<(), ProofError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(ProofError::Rejected);
    }
    send.write_all(&(bytes.len() as u16).to_be_bytes())
        .await
        .map_err(|_| ProofError::Transport)?;
    send.write_all(bytes)
        .await
        .map_err(|_| ProofError::Transport)
}

async fn read_frame(recv: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>, ProofError> {
    let mut len = [0; 2];
    recv.read_exact(&mut len)
        .await
        .map_err(|_| ProofError::Transport)?;
    let len = usize::from(u16::from_be_bytes(len));
    if len == 0 || len > MAX_FRAME {
        return Err(ProofError::Rejected);
    }
    let mut bytes = vec![0; len];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|_| ProofError::Transport)?;
    Ok(bytes)
}

// The server's iteration count and salt are untrusted until the final proof. Limit PBKDF2 before
// calling into rsasl, which otherwise accepts any nonzero u32 count.
fn acceptable_server_first(bytes: &[u8]) -> bool {
    let Ok(message) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut parts = message.split(',');
    matches!(parts.next(), Some(nonce) if nonce.starts_with("r=") && nonce.len() <= 96)
        && matches!(parts.next(), Some(salt) if salt.starts_with("s=") && salt.len() == 46)
        && parts.next() == Some("i=16384")
        && parts.next().is_none()
}

pub async fn prove_client(
    connection: &Connection,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<BootstrapProof, ProofError> {
    let _permit = ACTIVE_PROOFS.try_acquire().map_err(|_| ProofError::Busy)?;
    let attempt = reserve_once(connection)?;
    tokio::time::timeout(PROOF_TIMEOUT, async {
        let binding = exporter(connection)?;
        let sasl = SASLClient::with_cb(
            config(ClientCredentials {
                auth_id: scope.auth_id(),
                password: secret.scram_password(),
            })?,
            FixedBinding(binding),
        );
        let mut session = sasl
            .start_suggested(&[SCRAM_SHA256_PLUS.mechanism])
            .map_err(|_| ProofError::Rejected)?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|_| ProofError::Transport)?;
        let mut first = Vec::new();
        if !session
            .step(None, &mut first)
            .map_err(|_| ProofError::Rejected)?
            .is_running()
        {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, &first).await?;
        let server_first = read_frame(&mut recv).await?;
        if !acceptable_server_first(&server_first) {
            return Err(ProofError::Rejected);
        }
        let mut final_message = Vec::new();
        if !session
            .step(Some(&server_first), &mut final_message)
            .map_err(|_| ProofError::Rejected)?
            .is_running()
        {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, &final_message).await?;
        let server_final = read_frame(&mut recv).await?;
        let mut unused = Vec::new();
        if !matches!(
            session.step(Some(&server_final), &mut unused),
            Ok(State::Finished(_))
        ) {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, b"OK").await?;
        if read_frame(&mut recv).await? != b"OK" {
            return Err(ProofError::Rejected);
        }
        send.finish().map_err(|_| ProofError::Transport)?;
        Ok::<(), ProofError>(())
    })
    .await
    .map_err(|_| ProofError::Timeout)??;
    attempt.succeed(scope)
}

pub async fn prove_server(
    connection: &Connection,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<BootstrapProof, ProofError> {
    let _permit = ACTIVE_PROOFS.try_acquire().map_err(|_| ProofError::Busy)?;
    let attempt = reserve_once(connection)?;
    tokio::time::timeout(PROOF_TIMEOUT, async {
        let binding = exporter(connection)?;
        let sasl = SASLServer::<ProofValidation, _>::with_cb(
            config(ServerCredentials::new(scope, secret))?,
            FixedBinding(binding),
        );
        let mut session = sasl
            .start_suggested(SCRAM_SHA256_PLUS.mechanism)
            .map_err(|_| ProofError::Rejected)?;
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|_| ProofError::Transport)?;
        let first = read_frame(&mut recv).await?;
        if !first.starts_with(b"p=tls-exporter,,n=") {
            return Err(ProofError::Rejected);
        }
        let mut server_first = Vec::new();
        if !session
            .step(Some(&first), &mut server_first)
            .map_err(|_| ProofError::Rejected)?
            .is_running()
        {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, &server_first).await?;
        let client_final = read_frame(&mut recv).await?;
        let mut server_final = Vec::new();
        let state = session
            .step(Some(&client_final), &mut server_final)
            .map_err(|_| ProofError::Rejected)?;
        if !matches!(state, State::Finished(_)) {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, &server_final).await?;
        if session.validation() != Some(true) {
            return Err(ProofError::Rejected);
        }
        if read_frame(&mut recv).await? != b"OK" {
            return Err(ProofError::Rejected);
        }
        send_frame(&mut send, b"OK").await?;
        send.finish().map_err(|_| ProofError::Transport)?;
        Ok::<(), ProofError>(())
    })
    .await
    .map_err(|_| ProofError::Timeout)??;
    attempt.succeed(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scram_sha256_matches_rfc_7677_server_verifier() {
        // RFC 7677 §3's independent SCRAM-SHA-256 example, with no channel binding.
        // This checks the pinned rsasl hash/signature implementation against published bytes.
        let salt = hex::decode("5b6d99689d12358eeca04b141236fa81").unwrap();
        let mut salted = sha2::digest::Output::<Sha256>::default();
        tools::hash_password::<Sha256>(b"pencil", 4096, &salt, &mut salted);
        let (client_key, server_key) = tools::derive_keys::<Sha256>(&salted);
        let stored_key = Sha256::digest(client_key);
        let mut client_signature = sha2::digest::Output::<Sha256>::default();
        let mut server_signature = sha2::digest::Output::<Sha256>::default();
        tools::compute_signatures::<Sha256>(
            &stored_key,
            &server_key,
            "user",
            b"rOprNGfwEbeRWgbNEkqO",
            b"%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0",
            b"W22ZaJ0SNY7soEsUEjb6gQ==",
            b"4096",
            b"biws",
            &mut client_signature,
            &mut server_signature,
        );
        assert_eq!(
            hex::encode(server_signature),
            "eabae24d1062db75a9451ff0b6ea7e98c8546549ff741e672d3251b2397de46e"
        );
    }
}
