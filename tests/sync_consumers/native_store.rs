use p2panda_core::logs::LogHeights;
use p2panda_core::{Cursor, SigningKey, VerifyingKey};
use p2panda_store::cursors::CursorStore;
use p2panda_store::{SqliteStoreBuilder, tx};
use p2panda_sync as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("owned database path");
    let url = format!("sqlite://{path}");
    let store = SqliteStoreBuilder::new().database_url(&url).build().await?;
    let author = SigningKey::from_bytes(&[1; 32]).verifying_key();
    let mut cursor = Cursor::<VerifyingKey, u64>::new("legacy-sync-consumer", LogHeights::default());
    cursor.advance(author, 7, 42);
    tx!(store, { store.set_cursor(&cursor).await? });
    store.pool().close().await;
    drop(store);

    let reopened = SqliteStoreBuilder::new().database_url(&url).build().await?;
    let saved: Cursor<VerifyingKey, u64> = reopened.get_cursor("legacy-sync-consumer").await?.unwrap();
    assert_eq!(saved.log_height(&author, &7), Some(&42));
    assert_eq!(saved, cursor);
    reopened.pool().close().await;
    drop(reopened);
    println!("legacy transitive SQLite/macros transaction and durable reopen passed");
    Ok(())
}
