use std::{path::Path, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::Result;

/// Opens the database, applies the migrations, and hands back a (write, read)
/// pair of pools. The pragmas live here because they are per connection, not
/// per database: `foreign_keys` in particular is off by default, and without
/// it ON DELETE CASCADE silently does nothing.
pub async fn connect(db_path: &Path) -> Result<(SqlitePool, SqlitePool)> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    // SQLite allows one writer at a time. Capping the pool at one connection
    // makes that queue explicit instead of letting it surface as SQLITE_BUSY.
    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await?;

    let read = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&write).await?;

    return Ok((write, read));
}
