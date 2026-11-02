//! Join SQLite connection workers before publishing or removing their files.

pub(crate) async fn close(pool: &sqlx::SqlitePool) {
    // SQLx 0.8.6 can return from close after acquiring its final permit even
    // when an in-flight return just put a connection back in the idle queue.
    // A closed pool accepts no new acquisitions. Drain that late return too;
    // graceful connection close joins the SQLite worker and releases its files.
    loop {
        pool.close().await;
        if pool.size() == 0 {
            break;
        }
    }
}
