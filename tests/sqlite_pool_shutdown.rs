//! Real SQLite shutdown with a connection returning while the pool closes.
//! The public SQLx callback fixes the scheduling boundary; no process or clock
//! is replaced. Both outcomes settle the worker before removing the fixture.
#[path = "../src/sqlite_shutdown.rs"]
mod sqlite_shutdown;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tokio::sync::{Notify, Semaphore};

#[tokio::test]
async fn close_joins_a_connection_returning_during_shutdown() {
    let directory = std::env::temp_dir().join(format!(
        "miopunch-sqlite-close-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_release({
            let entered = entered.clone();
            let release = release.clone();
            move |_, _| {
                let entered = entered.clone();
                let release = release.clone();
                Box::pin(async move {
                    entered.notify_one();
                    release.acquire().await.unwrap().forget();
                    Ok(true)
                })
            }
        })
        .connect_with(
            SqliteConnectOptions::new()
                .filename(directory.join("state.sqlite"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let connection = pool.acquire().await.unwrap();
    drop(connection);
    entered.notified().await;
    let closing = sqlite_shutdown::close(&pool);
    tokio::pin!(closing);
    tokio::select! {
        biased;
        _ = &mut closing => panic!("close returned while its connection was still borrowed"),
        _ = std::future::ready(()) => (),
    }
    assert!(pool.is_closed());
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), &mut closing)
        .await
        .unwrap();
    let remaining = pool.size();
    // Settle even the original failing implementation before the assertion.
    pool.close().await;
    assert_eq!(pool.size(), 0);
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(remaining, 0, "closed pool retained a live SQLite worker");
}

#[tokio::test]
async fn close_joins_idle_connections_and_is_idempotent() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlite_shutdown::close(&pool).await;
    assert!(pool.is_closed());
    assert_eq!(pool.size(), 0);
    sqlite_shutdown::close(&pool).await;
    assert_eq!(pool.size(), 0);
}
