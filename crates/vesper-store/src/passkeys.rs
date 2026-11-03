//! Passkey rows. The store keeps the credential as the opaque JSON text the server gives it; it
//! never parses it.

use sqlx::FromRow;
use vesper_protocol::PasskeyInfo;

use crate::{Conflict, Result, Store, StoreError, constraint};

/// Passkeys per account.
pub const MAX_PASSKEYS: i64 = 10;

/// What a passkey sign in needs. `credential` is a public key and a counter; it is still
/// not logged.
#[derive(FromRow)]
pub struct PasskeyAuth {
    /// Row id.
    pub id: i64,
    /// Owner account id.
    pub account_id: String,
    /// The stored credential JSON.
    pub credential: String,
    /// The owner is banned: the sign in must fail.
    pub banned: bool,
}

impl Store {
    /// Adds a passkey. `Conflict(TooManyPasskeys)` past [`MAX_PASSKEYS`],
    /// `Conflict(PasskeyExists)` when `credential_id` is registered already, `NotFound` when the
    /// account is gone.
    pub async fn add_passkey(
        &self,
        account_id: &str,
        credential_id: &[u8],
        credential: &str,
        name: &str,
        now: i64,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM passkeys WHERE account_id = ?1")
            .bind(account_id)
            .fetch_one(&mut *tx)
            .await?;
        if n >= MAX_PASSKEYS {
            return Err(StoreError::Conflict(Conflict::TooManyPasskeys));
        }
        let id = sqlx::query_scalar(
            "INSERT INTO passkeys (account_id, credential_id, credential, name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
        )
        .bind(account_id)
        .bind(credential_id)
        .bind(credential)
        .bind(name)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| constraint(e, Conflict::PasskeyExists))?;
        tx.commit().await?;
        Ok(id)
    }

    /// This account's passkeys, oldest first.
    pub async fn passkeys(&self, account_id: &str) -> Result<Vec<PasskeyInfo>> {
        #[derive(FromRow)]
        struct Row {
            id: i64,
            name: String,
            created_at: i64,
            last_used_at: Option<i64>,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, name, created_at, last_used_at FROM passkeys
             WHERE account_id = ?1 ORDER BY id",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| PasskeyInfo {
                id: r.id,
                name: r.name,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
            })
            .collect())
    }

    /// The stored credential JSON of every passkey of the account (for the exclude list).
    pub async fn passkey_credentials(&self, account_id: &str) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT credential FROM passkeys WHERE account_id = ?1 ORDER BY id")
                .bind(account_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// The passkey with this credential id, with its owner's ban flag.
    pub async fn passkey_by_credential(&self, credential_id: &[u8]) -> Result<Option<PasskeyAuth>> {
        Ok(sqlx::query_as(
            "SELECT p.id, p.account_id, p.credential, a.banned_at IS NOT NULL AS banned
             FROM passkeys p JOIN accounts a ON a.id = p.account_id
             WHERE p.credential_id = ?1",
        )
        .bind(credential_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Stores the credential after a sign in (the counter moved) and marks it used.
    pub async fn passkey_used(&self, id: i64, credential: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE passkeys SET credential = ?2, last_used_at = ?3 WHERE id = ?1")
            .bind(id)
            .bind(credential)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `NotFound` unless passkey `id` belongs to `account_id`.
    pub async fn rename_passkey(&self, account_id: &str, id: i64, name: &str) -> Result<()> {
        let n = sqlx::query("UPDATE passkeys SET name = ?3 WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// `NotFound` unless passkey `id` belongs to `account_id`.
    pub async fn delete_passkey(&self, account_id: &str, id: i64) -> Result<()> {
        let n = sqlx::query("DELETE FROM passkeys WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
