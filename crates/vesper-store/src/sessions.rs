use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::{Device, UserInfo};

use crate::accounts::{AccountRow, user_cols};
use crate::{Result, Store, StoreError, constraint};

/// A session dies 90 days after its last use.
pub const SESSION_TTL_MS: i64 = 90 * 24 * 3600 * 1000;
/// `last_seen_at` is written at most this often.
const TOUCH_MS: i64 = 3600 * 1000;

pub struct NewSession<'a> {
    pub token_hash: [u8; 32],
    pub label: &'a str,
    pub now: i64,
}

pub(crate) async fn insert_session(
    c: &mut SqliteConnection,
    user_id: &str,
    s: &NewSession<'_>,
) -> Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO sessions (account_id, token_hash, label, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
    )
    .bind(user_id)
    .bind(&s.token_hash[..])
    .bind(s.label)
    .bind(s.now)
    .fetch_one(c)
    .await
    .map_err(|e| constraint(e, "token exists"))
}

#[derive(FromRow)]
struct SessionUser {
    session_id: i64,
    last_seen_at: i64,
    #[sqlx(flatten)]
    account: AccountRow,
}

impl Store {
    pub async fn create_session(&self, user_id: &str, s: NewSession<'_>) -> Result<i64> {
        insert_session(&mut *self.pool.acquire().await?, user_id, &s).await
    }

    /// The live session and its user. None when the token is unknown, unused for longer than
    /// `SESSION_TTL_MS`, or the user is banned.
    pub async fn session_user(
        &self,
        token_hash: &[u8; 32],
        now: i64,
    ) -> Result<Option<(i64, UserInfo)>> {
        let row: Option<SessionUser> = sqlx::query_as(concat!(
            "SELECT s.id AS session_id, s.last_seen_at, ",
            user_cols!(),
            " FROM sessions s JOIN accounts a ON a.id = s.account_id WHERE s.token_hash = ?1"
        ))
        .bind(&token_hash[..])
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        if now - row.last_seen_at > SESSION_TTL_MS {
            sqlx::query("DELETE FROM sessions WHERE id = ?1")
                .bind(row.session_id)
                .execute(&self.pool)
                .await?;
            return Ok(None);
        }
        let user = row.account.into_info()?;
        if user.banned {
            return Ok(None);
        }
        if now - row.last_seen_at > TOUCH_MS {
            sqlx::query("UPDATE sessions SET last_seen_at = ?2 WHERE id = ?1")
                .bind(row.session_id)
                .bind(now)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some((row.session_id, user)))
    }

    /// The user's devices; `current` marks the session id passed in.
    pub async fn sessions(&self, user_id: &str, current: i64) -> Result<Vec<Device>> {
        let rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(
            "SELECT id, label, created_at, last_seen_at FROM sessions
             WHERE account_id = ?1 ORDER BY last_seen_at DESC, id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, label, created_at, last_seen_at)| Device {
                id,
                label,
                created_at,
                last_seen_at,
                current: id == current,
            })
            .collect())
    }

    /// NotFound unless session `id` belongs to `user_id`.
    pub async fn delete_session(&self, user_id: &str, id: i64) -> Result<()> {
        let n = sqlx::query("DELETE FROM sessions WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
