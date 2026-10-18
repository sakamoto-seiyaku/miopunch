//! Public selector entry points exercised against two real same-peer connections.
use iroh::{
    Endpoint, RelayMode,
    endpoint::{
        Connection, presets,
        transports::{ConnectionPathSelector, PathSelection, PathSelectionContext, PathSelector},
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Default)]
struct Recorder(Mutex<Vec<(Option<usize>, usize, usize)>>, AtomicBool);
impl Recorder {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        let paths: Vec<_> = ctx.paths().collect();
        let exact_current = paths.iter().filter(|p| ctx.is_current(p)).count();
        // The old current() remains a real address. Exact equality is a
        // separate operation and includes the private local socket binding.
        if exact_current > 0 {
            assert!(
                paths
                    .iter()
                    .any(|p| ctx.is_current(p) && ctx.current() == Some(p.network_path()))
            );
        }
        if !self.1.load(Ordering::SeqCst) {
            self.0
                .lock()
                .unwrap()
                .push((ctx.connection_id(), paths.len(), exact_current));
        }
        let mut selection = PathSelection::none();
        if let Some(path) = paths.first() {
            selection.set(path);
        }
        selection
    }
}
impl ConnectionPathSelector for Recorder {
    fn reevaluation_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        self.select(ctx)
    }
}
#[derive(Debug)]
struct Legacy(Arc<Recorder>);
impl PathSelector for Legacy {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        self.0.select(ctx)
    }
}
async fn ping(conn: &Connection) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"PONG");
}

async fn exercise(legacy_last: bool, product_default: bool) {
    let scoped = Arc::new(Recorder::default());
    let legacy = Arc::new(Recorder::default());
    let server = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![b"m101/scoped".to_vec()])
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap();
    let mut builder = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Disabled)
        .bind_addr("127.0.0.1:0")
        .unwrap();
    builder = if legacy_last {
        builder
            .connection_path_selector(scoped.clone())
            .path_selector(Arc::new(Legacy(legacy.clone())))
    } else {
        builder
            .path_selector(Arc::new(Legacy(legacy.clone())))
            .connection_path_selector(scoped.clone())
    };
    if product_default {
        // Installing saved product defaults must preserve either explicit entry.
        builder = builder.default_connection_path_selector(Arc::new(Recorder::default()));
    }
    let client = builder.bind().await.unwrap();
    let task = tokio::spawn({
        let server = server.clone();
        async move {
            let mut workers = Vec::new();
            for _ in 0..2 {
                let conn = server.accept().await.unwrap().await.unwrap();
                workers.push(tokio::spawn(async move {
                    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        assert_eq!(recv.read_to_end(16).await.unwrap(), b"PING");
                        send.write_all(b"PONG").await.unwrap();
                        send.finish().unwrap();
                    }
                }));
            }
            for worker in workers {
                worker.await.unwrap();
            }
        }
    });
    let a = client.connect(server.addr(), b"m101/scoped").await.unwrap();
    ping(&a).await;
    let b = client.connect(server.addr(), b"m101/scoped").await.unwrap();
    let ids = [a.stable_id(), b.stable_id()];
    assert_ne!(ids[0], ids[1]);
    for _ in 0..4 {
        ping(&a).await;
        ping(&b).await;
        tokio::time::sleep(Duration::from_millis(90)).await;
    }
    let paths = [&a, &b].map(|c| {
        c.paths()
            .iter()
            .filter(|p| p.is_selected())
            .map(|p| format!("{:?}", p.remote_addr()))
            .collect::<Vec<_>>()
    });
    scoped.1.store(true, Ordering::SeqCst);
    legacy.1.store(true, Ordering::SeqCst);
    a.close(0u32.into(), b"done");
    b.close(0u32.into(), b"done");
    task.await.unwrap();
    client.close().await;
    server.close().await;
    let scoped = scoped.0.lock().unwrap();
    let legacy = legacy.0.lock().unwrap();
    println!("stable_ids={ids:?} scoped={scoped:?} legacy={legacy:?} selected={paths:?} CLOSED");
    assert!(paths.iter().all(|p| p.len() == 1));
    if legacy_last {
        assert!(scoped.is_empty());
        assert_eq!(legacy.iter().map(|r| r.1).collect::<Vec<_>>(), vec![1, 2]);
        assert!(legacy.iter().all(|r| r.0.is_none()));
    } else {
        assert!(legacy.is_empty());
        assert!(
            scoped
                .iter()
                .all(|r| r.0.is_some_and(|id| ids.contains(&id)) && r.1 == 1 && r.2 <= 1)
        );
        for id in ids {
            assert!(scoped.iter().filter(|r| r.0 == Some(id)).count() >= 2);
            assert!(scoped.iter().any(|r| r.0 == Some(id) && r.2 == 1));
        }
    }
}
#[tokio::test]
async fn connection_callbacks_never_receive_the_peer_aggregate() {
    tokio::time::timeout(Duration::from_secs(8), exercise(false, false))
        .await
        .unwrap();
}
#[tokio::test]
async fn product_defaults_keep_explicit_connection_policy() {
    tokio::time::timeout(Duration::from_secs(8), exercise(false, true))
        .await
        .unwrap();
}
#[tokio::test]
async fn product_defaults_keep_legacy_policy_and_last_registration_wins() {
    tokio::time::timeout(Duration::from_secs(8), exercise(true, true))
        .await
        .unwrap();
}
