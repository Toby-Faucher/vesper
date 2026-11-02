#![forbid(unsafe_code)]
//! SQLite storage (ADR 0005). The only crate that imports sqlx. It sees token and code hashes,
//! never the tokens or codes themselves.

use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use vesper_protocol::Role;

mod accounts;
mod invites;
mod messages;
mod resets;
mod rooms;
mod search;
mod sessions;
mod uploads;

pub use accounts::{LoginRow, NewAccount};
pub use messages::{Cursor, MAX_PINS, MAX_REACTIONS, NewMessage};
pub use resets::RESET_TTL_MS;
pub use search::{HITS as SEARCH_SQL, MAX_SEARCH, fts_query};
pub use sessions::{NewSession, SESSION_TTL_MS};
pub use uploads::UploadRow;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug)]
pub enum StoreError {
    NotFound,
    Conflict(&'static str),
    InviteInvalid,
    Db(sqlx::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound => f.write_str("not found"),
            StoreError::Conflict(what) => write!(f, "conflict: {what}"),
            StoreError::InviteInvalid => f.write_str("invite not valid"),
            StoreError::Db(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        StoreError::Db(e)
    }
}

/// New data dir 0700 and new db file 0600 (the -wal and -shm files copy the db file's mode).
/// An existing dir keeps its mode, so a bind mount keeps its host permissions.
fn create_private(dir: &Path, db: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(db)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    let _ = db;
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (or creates) the database, creating the parent directory, and runs the migrations.
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private(dir, path).map_err(|e| StoreError::Db(sqlx::Error::Io(e)))?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(|e| StoreError::Db(e.into()))?;
        Ok(Store { pool })
    }

    /// Raw pool, for this crate's tests only. Hidden so the server does not reach sqlx (ADR 0005).
    #[doc(hidden)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// `bytes` random bytes from the OS as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random source failed");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Unix time in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::Owner => "owner",
        Role::Moderator => "moderator",
        Role::Member => "member",
    }
}

fn parse_role(s: &str) -> Result<Role> {
    match s {
        "owner" => Ok(Role::Owner),
        "moderator" => Ok(Role::Moderator),
        "member" => Ok(Role::Member),
        _ => Err(StoreError::Db(sqlx::Error::Decode(
            format!("unknown role {s:?}").into(),
        ))),
    }
}

/// Maps a unique-constraint failure to `Conflict(what)` and a foreign key failure to `NotFound`.
fn constraint(e: sqlx::Error, what: &'static str) -> StoreError {
    if let sqlx::Error::Database(d) = &e {
        if d.is_unique_violation() {
            return StoreError::Conflict(what);
        }
        if d.is_foreign_key_violation() {
            return StoreError::NotFound;
        }
    }
    StoreError::Db(e)
}
