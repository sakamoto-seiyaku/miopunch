//! One native node owned by a platform host, independently of its windows.
//! Closing a view never calls stop. Explicit stop cancels active commands,
//! waits for their borrows, and closes the actual SDK before acknowledging.
use std::{
    fs::File,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::{
    client_commands::{self, Command, Request},
    native_client::{ClientError, ClientInvitation, NativeClient},
    resources::ResourceConfig,
};

/// Start information is private: joining material must not enter diagnostics.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum StartMode {
    Create,
    Open,
    Join { invitation: String },
    JoinGuest { invitation: String },
}

struct Session {
    client: RwLock<Option<NativeClient>>,
    cancelled: CancellationToken,
    // Application-host exclusion only; not a distributed or physical clone lock.
    _directory_lock: HostLease,
}

/// Cooperative exclusion across installed hosts and the application CLI.
/// The sibling lock survives node replacement; never unlink it on shutdown.
/// This is not protection against a copied identity or an untrusted local user.
pub struct HostLease {
    directory: PathBuf,
    _lock: File,
}

impl HostLease {
    pub fn acquire(directory: impl AsRef<Path>) -> Result<Self, ClientError> {
        let requested = directory.as_ref();
        let directory = if requested.exists() {
            requested.canonicalize()?
        } else {
            let parent = requested
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            std::fs::create_dir_all(parent)?;
            parent
                .canonicalize()?
                .join(requested.file_name().ok_or("node directory has no name")?)
        };
        let parent = directory.parent().ok_or("node directory has no parent")?;
        let mut name = directory
            .file_name()
            .ok_or("node directory has no name")?
            .to_os_string();
        name.push(".host-lock");
        let mut options = File::options();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(parent.join(name))?;
        lock.try_lock()
            .map_err(|_| "node already has an application host")?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// The shell owns this object for its entire process/service lifetime.
#[derive(Default)]
pub struct ClientHost {
    session: Arc<Mutex<Option<Arc<Session>>>>,
}

impl ClientHost {
    pub async fn start(
        &self,
        directory: impl AsRef<Path>,
        mode: StartMode,
        resources: ResourceConfig,
    ) -> Result<Value, ClientError> {
        let mut owner = self.session.lock().await;
        if owner.is_some() {
            return Err("this host already owns a node".into());
        }
        let directory_lock = HostLease::acquire(directory)?;
        let directory = directory_lock.directory();
        let client = match mode {
            StartMode::Create => {
                NativeClient::create_with_resources(directory, Ipv4Addr::UNSPECIFIED, resources)
                    .await?
            }
            StartMode::Open => NativeClient::open(directory, Ipv4Addr::UNSPECIFIED).await?,
            StartMode::Join { invitation } => {
                NativeClient::join_with_resources(
                    directory,
                    &ClientInvitation::decode(invitation.as_bytes())?,
                    Ipv4Addr::UNSPECIFIED,
                    resources,
                )
                .await?
            }
            StartMode::JoinGuest { invitation } => {
                NativeClient::join_guest_with_resources(
                    directory,
                    &crate::service_policy::GuestInvitation::decode(invitation.as_bytes())?,
                    Ipv4Addr::UNSPECIFIED,
                    resources,
                )
                .await?
            }
        };
        let ready = json!({"event":"ready", "status":client_commands::status(&client).await,
            "address":client.address()});
        *owner = Some(Arc::new(Session {
            client: RwLock::new(Some(client)),
            cancelled: CancellationToken::new(),
            _directory_lock: directory_lock,
        }));
        Ok(ready)
    }

    pub async fn running(&self) -> bool {
        self.session.lock().await.is_some()
    }

    /// Bounded trusted control input, shared with the existing CLI vocabulary.
    /// Network permissions continue to be enforced by NativeClient itself.
    pub async fn request(&self, input: &str) -> Value {
        if input.len() > 65536 {
            return json!({"id":null,"ok":false,"error":"invalid_command"});
        }
        let Ok(request) = serde_json::from_str::<Request>(input) else {
            return json!({"id":null,"ok":false,"error":"invalid_command"});
        };
        let result = match request.command {
            Command::Stop => self.stop().await.map(|()| json!({"stopped":true})),
            Command::ReplacementPrepare { path } => self.replace(&path).await,
            command => self.execute(command).await,
        };
        match result {
            Ok(value) => json!({"id":request.id,"ok":true,"result":value}),
            Err(_) => json!({"id":request.id,"ok":false,"error":"operation_failed"}),
        }
    }

    async fn execute(&self, command: Command) -> Result<Value, ClientError> {
        let session = self.session.lock().await.clone().ok_or("host is stopped")?;
        let node = session.client.read().await;
        let client = node.as_ref().ok_or("host is stopped")?;
        tokio::select! {
            biased;
            _ = session.cancelled.cancelled() => Err("host is stopping".into()),
            result = client_commands::dispatch(client, command) => result,
        }
    }

    pub async fn stop(&self) -> Result<(), ClientError> {
        let mut owner = self.session.clone().lock_owned().await;
        let Some(session) = owner.take() else {
            return Ok(());
        };
        session.cancelled.cancel();
        // Retain lifecycle ownership until close completes even if a window's
        // request future disappears. A new start waits on this same mutex.
        tokio::spawn(async move {
            let client = session.client.write().await.take();
            let result = match client {
                Some(client) => client.close().await,
                None => Ok(()),
            };
            drop(session);
            drop(owner);
            result
        })
        .await?
    }

    async fn replace(&self, destination: &str) -> Result<Value, ClientError> {
        let mut owner = self.session.clone().lock_owned().await;
        let session = owner.take().ok_or("host is stopped")?;
        session.cancelled.cancel();
        let destination = destination.to_owned();
        tokio::spawn(async move {
            let client = session
                .client
                .write()
                .await
                .take()
                .ok_or("host is stopped")?;
            let result = client.prepare_node_replacement(destination).await;
            drop(session);
            drop(owner);
            result
        })
        .await?
    }
}
