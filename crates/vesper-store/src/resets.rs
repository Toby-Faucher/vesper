use crate::{Result, Store, constraint, now_ms};

/// A reset code works for 24 hours.
pub const RESET_TTL_MS: i64 = 24 * 3600 * 1000;

impl Store {
    /// Sets the account's one reset code, replacing any older one. NotFound for an unknown account.
    pub async fn create_reset(
        &self,
        account_id: &str,
        code_hash: &[u8; 32],
        expires_at: i64,
        created_by: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO password_resets (account_id, code_hash, expires_at, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (account_id) DO UPDATE SET code_hash = excluded.code_hash,
             expires_at = excluded.expires_at, created_by = excluded.created_by,
             created_at = excluded.created_at",
        )
        .bind(account_id)
        .bind(&code_hash[..])
        .bind(expires_at)
        .bind(created_by)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .map_err(|e| constraint(e, "reset code exists"))?;
        Ok(())
    }

    /// True while the code is unused and unexpired. A cheap check before the argon2 hash.
    pub async fn reset_valid(&self, code_hash: &[u8; 32], now: i64) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM password_resets WHERE code_hash = ?1 AND expires_at > ?2)",
        )
        .bind(&code_hash[..])
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Uses the code: sets the password, deletes the code and every session of the account, in
    /// one transaction. Returns the account id, or None when the code is unknown, used or expired.
    pub async fn use_reset(
        &self,
        code_hash: &[u8; 32],
        phc: &str,
        now: i64,
    ) -> Result<Option<String>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let id: Option<String> = sqlx::query_scalar(
            "DELETE FROM password_resets WHERE code_hash = ?1 AND expires_at > ?2
             RETURNING account_id",
        )
        .bind(&code_hash[..])
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(id) = id else { return Ok(None) };
        sqlx::query("UPDATE accounts SET password_phc = ?2 WHERE id = ?1")
            .bind(&id)
            .bind(phc)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE account_id = ?1")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(id))
    }
}
