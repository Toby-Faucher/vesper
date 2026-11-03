#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! SQLite storage (ADR 0005). The only crate that imports sqlx. It sees token and code hashes,
//! never the tokens or codes themselves.

use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use vesper_protocol::Role;

mod accounts;
mod calls;
mod invites;
mod messages;
mod passkeys;
mod push;
mod resets;
mod rooms;
mod search;
mod sessions;
mod uploads;

pub use accounts::{LoginRow, NewAccount};
pub use calls::{CALLS_KEPT, CALLS_TTL_MS};
pub use messages::{Cursor, MAX_PINS, MAX_REACTIONS, NewMessage};
pub use passkeys::{MAX_PASSKEYS, PasskeyAuth};
pub use push::{MAX_PUSH_DEVICES, NewPushSub, PushKind, PushTarget};
pub use resets::RESET_TTL_MS;
#[doc(hidden)] // test seam: the search tests EXPLAIN this query
pub use search::HITS as SEARCH_SQL;
pub use search::{MAX_SEARCH, fts_query};
pub use sessions::{NewSession, SESSION_TTL_MS};
pub use uploads::UploadRow;

/// Result of a store call.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Which rule a change broke. `Display` is the text the user sees; callers match on the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// The last live room cannot be archived.
    LastRoom,
    /// A room reorder must list every live room exactly once.
    OrderIncomplete,
    /// A direct message needs two different people.
    DmSelf,
    /// A reply must point at a message in the same room.
    ReplyOutsideRoom,
    /// An attachment is missing, not the author's, in another room or already used.
    AttachmentUnavailable,
    /// A message already has `MAX_REACTIONS` different reactions.
    TooManyReactions,
    /// A room already has `MAX_PINS` pins.
    PinCap,
    /// A room with that name exists.
    NameTaken,
    /// The account already has `MAX_PUSH_DEVICES` push devices.
    TooManyDevices,
    /// The push endpoint belongs to another session.
    EndpointTaken,
    /// An invite with that code hash exists.
    InviteExists,
    /// A reset link with that code hash exists.
    ResetCodeExists,
    /// The account is already a member of the room.
    DuplicateMember,
    /// The account does not exist.
    NoSuchAccount,
    /// The same account is mentioned twice in one message.
    DuplicateMention,
    /// A message with that id exists.
    MessageExists,
    /// The message was deleted and cannot be changed.
    MessageDeleted,
    /// The account already added that reaction.
    DuplicateReaction,
    /// Storing the pin broke a database constraint.
    PinFailed,
    /// Storing the read marker broke a database constraint.
    ReadMarker,
    /// A session with that token hash exists.
    TokenExists,
    /// An upload with that id exists.
    UploadExists,
    /// The username is taken.
    UsernameTaken,
    /// The account already has `MAX_PASSKEYS` passkeys.
    TooManyPasskeys,
    /// That passkey is already registered.
    PasskeyExists,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Conflict::LastRoom => "the last room cannot be archived",
            Conflict::OrderIncomplete => "the order must list every room once",
            Conflict::DmSelf => "a DM needs two different people",
            Conflict::ReplyOutsideRoom => "reply must be in the same room",
            Conflict::AttachmentUnavailable => "attachment not available",
            Conflict::TooManyReactions => "too many different reactions",
            Conflict::PinCap => "this room has 50 pins",
            Conflict::NameTaken => "a room with that name exists",
            Conflict::TooManyDevices => "too many push devices",
            Conflict::EndpointTaken => "push endpoint belongs to another session",
            Conflict::InviteExists => "invite exists",
            Conflict::ResetCodeExists => "reset code exists",
            Conflict::DuplicateMember => "duplicate member",
            Conflict::NoSuchAccount => "no such account",
            Conflict::DuplicateMention => "duplicate mention",
            Conflict::MessageExists => "message exists",
            Conflict::MessageDeleted => "message deleted",
            Conflict::DuplicateReaction => "duplicate reaction",
            Conflict::PinFailed => "pin",
            Conflict::ReadMarker => "read marker",
            Conflict::TokenExists => "token exists",
            Conflict::UploadExists => "upload exists",
            Conflict::UsernameTaken => "username taken",
            Conflict::TooManyPasskeys => "too many passkeys",
            Conflict::PasskeyExists => "that passkey is already registered",
        })
    }
}

/// What a store call can fail with. Print it with `{}`, or walk [`std::error::Error::source`] to
/// reach the `sqlx::Error` or `io::Error` underneath. A caller can act on the first three; `Db`
/// and `Io` mean the server is at fault.
#[derive(Debug)]
pub enum StoreError {
    /// The row asked for is not there (also a failed foreign key).
    NotFound,
    /// The change breaks a rule (a duplicate, a cap). Its `Display` is meant for the user.
    Conflict(Conflict),
    /// The invite code is unknown, used, expired or revoked.
    InviteInvalid,
    /// The database failed, or returned a value this binary can not read.
    Db(sqlx::Error),
    /// The data directory or a backup path could not be used.
    Io(std::io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound => f.write_str("not found"),
            StoreError::Conflict(what) => write!(f, "conflict: {what}"),
            StoreError::InviteInvalid => f.write_str("invite not valid"),
            StoreError::Db(e) => write!(f, "database error: {e}"),
            StoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Db(e) => Some(e),
            StoreError::Io(e) => Some(e),
            StoreError::NotFound | StoreError::Conflict(_) | StoreError::InviteInvalid => None,
        }
    }
}

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

/// Handle to the database. Cheap to clone; clones share one connection pool.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (or creates) the database, creating the parent directory, and runs the migrations.
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private(dir, path).map_err(StoreError::Io)?;
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

    /// Opens an existing database as it is: never creates the file and never runs migrations
    /// (for `backup`, which must not touch a live database a newer binary could migrate).
    /// A missing file is an error.
    pub async fn open_existing(path: &Path) -> Result<Store> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .busy_timeout(Duration::from_millis(5000));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Ok(Store { pool })
    }

    /// The newest migration applied to the database at `path`, and the newest this binary
    /// carries, read without creating or changing anything. `None` when there is no database
    /// file yet. An empty file (or one never migrated) has applied 0. When the first is smaller,
    /// [`Store::open`] will migrate: the server takes a backup before that.
    pub async fn schema_versions(path: &Path) -> Result<Option<(i64, i64)>> {
        if !path.is_file() {
            return Ok(None);
        }
        let s = Store::open_existing(path).await?;
        let table: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(&s.pool)
        .await?;
        let applied: Option<i64> = if table == 0 {
            None
        } else {
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
                .fetch_one(&s.pool)
                .await?
        };
        s.pool.close().await;
        let embedded = sqlx::migrate!()
            .iter()
            .map(|m| m.version)
            .max()
            .unwrap_or(0);
        Ok(Some((applied.unwrap_or(0), embedded)))
    }

    /// A consistent copy of the whole database at `dest` (`VACUUM INTO`), safe while others write.
    /// `dest` must not exist or must be an empty file; SQLite refuses anything else.
    pub async fn backup_into(&self, dest: &Path) -> Result<()> {
        let dest = dest.to_str().ok_or_else(|| {
            StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "backup path is not UTF-8",
            ))
        })?;
        // Bound, never formatted into the SQL: SQLite 3.27+ takes an expression here.
        sqlx::query("VACUUM INTO ?1")
            .bind(dest)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A database migrated to the first `n` migrations only: the schema of an older release,
    /// for tests of the backup before a migration.
    #[doc(hidden)]
    pub async fn create_at_schema(path: &Path, n: usize) -> Result<()> {
        Store::migrate_first(path, n, false).await
    }

    /// The first `n` migrations, but number `n` is replaced by one that fails: an upgrade that
    /// stops part way (sqlx commits each migration on its own), for tests.
    #[doc(hidden)]
    pub async fn migrate_failing_at(path: &Path, n: usize) -> Result<()> {
        Store::migrate_first(path, n, true).await
    }

    async fn migrate_first(path: &Path, n: usize, fail_last: bool) -> Result<()> {
        use sqlx::SqlSafeStr;
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new().connect_with(opts).await?;
        let mut m = sqlx::migrate!();
        let mut list = m.migrations[..n].to_vec();
        if fail_last && let Some(last) = list.last_mut() {
            *last = sqlx::migrate::Migration::new(
                last.version,
                last.description.clone(),
                last.migration_type,
                "SELECT no_such_column FROM no_such_table".into_sql_str(),
                false,
            );
        }
        m.migrations = list.into();
        let r = m.run(&pool).await.map_err(|e| StoreError::Db(e.into()));
        pool.close().await;
        r
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
fn constraint(e: sqlx::Error, what: Conflict) -> StoreError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn source_reaches_the_cause() {
        let io = StoreError::Io(std::io::Error::other("disk"));
        assert_eq!(io.source().unwrap().to_string(), "disk");
        let db = StoreError::from(sqlx::Error::RowNotFound);
        assert!(db.source().unwrap().is::<sqlx::Error>());
        assert!(StoreError::NotFound.source().is_none());
    }
}
