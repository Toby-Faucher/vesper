use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::{ResetInfo, ResetNotice};

use crate::{Result, Store, constraint, now_ms};

/// A reset code works for 24 hours.
pub const RESET_TTL_MS: i64 = 24 * 3600 * 1000;

/// Cancels the account's live reset code, if any. An expired unused one stays as it is, so the
/// history still says expired. Called on a role change, a ban and a password change.
pub(crate) async fn cancel_reset(
    c: &mut SqliteConnection,
    account_id: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE password_resets SET cancelled_at = ?2
         WHERE account_id = ?1 AND used_at IS NULL AND cancelled_at IS NULL AND expires_at > ?2",
    )
    .bind(account_id)
    .bind(now)
    .execute(c)
    .await?;
    Ok(())
}

#[derive(FromRow)]
struct ResetRow {
    account_id: String,
    created_by: Option<String>,
    created_at: i64,
    expires_at: i64,
    used_at: Option<i64>,
    used_from: Option<String>,
    cancelled: bool,
}

/// Cancels the live links `maker` made for others: a banned or demoted moderator must not keep
/// working links. Called on every role change and ban or unban of `maker`.
pub(crate) async fn cancel_made_by(c: &mut SqliteConnection, maker: &str, now: i64) -> Result<()> {
    sqlx::query(
        "UPDATE password_resets SET cancelled_at = ?2
         WHERE created_by = ?1 AND used_at IS NULL AND cancelled_at IS NULL AND expires_at > ?2",
    )
    .bind(maker)
    .bind(now)
    .execute(c)
    .await?;
    Ok(())
}

/// Resets used this recently are shown again to the person after each new reset, and listed to
/// the owner.
const NOTICE_REARM_MS: i64 = 30 * 24 * 3600 * 1000;

impl Store {
    /// A new reset code for the account; any older unused one is cancelled. Rows are kept as the
    /// audit. NotFound for an unknown account.
    pub async fn create_reset(
        &self,
        account_id: &str,
        code_hash: &[u8; 32],
        expires_at: i64,
        created_by: Option<&str>,
    ) -> Result<()> {
        let now = now_ms();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE password_resets SET cancelled_at = ?2
             WHERE account_id = ?1 AND used_at IS NULL AND cancelled_at IS NULL",
        )
        .bind(account_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO password_resets (account_id, code_hash, expires_at, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(account_id)
        .bind(&code_hash[..])
        .bind(expires_at)
        .bind(created_by)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| constraint(e, "reset code exists"))?;
        tx.commit().await?;
        Ok(())
    }

    /// True while the code is unused, not cancelled and unexpired. A cheap check before argon2.
    pub async fn reset_valid(&self, code_hash: &[u8; 32], now: i64) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM password_resets WHERE code_hash = ?1
             AND used_at IS NULL AND cancelled_at IS NULL AND expires_at > ?2)",
        )
        .bind(&code_hash[..])
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Uses the code: marks it used (with `used_from`, `lan` or `internet`), sets the password and
    /// deletes every session of the account, in one transaction. Returns the account id and who
    /// made the link, or None when the code is unknown, used, cancelled or expired.
    pub async fn use_reset(
        &self,
        code_hash: &[u8; 32],
        phc: &str,
        now: i64,
        used_from: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "UPDATE password_resets SET used_at = ?2, used_from = ?3
             WHERE code_hash = ?1 AND used_at IS NULL AND cancelled_at IS NULL AND expires_at > ?2
             RETURNING account_id, created_by",
        )
        .bind(&code_hash[..])
        .bind(now)
        .bind(used_from)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, by)) = row else { return Ok(None) };
        // The taker of an account sees and uses up the notice first; when the real person gets
        // back in through a later link, they are told about every recent reset again.
        sqlx::query(
            "UPDATE password_resets SET seen_at = NULL WHERE account_id = ?1 AND used_at > ?2",
        )
        .bind(&id)
        .bind(now - NOTICE_REARM_MS)
        .execute(&mut *tx)
        .await?;
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
        Ok(Some((id, by)))
    }

    /// For each account with a reset: its latest row, plus every reset used in the 30 days before
    /// `now`, so a newer link (even the owner's recovery link) can not hide a used one. Newest first.
    pub async fn resets(&self, now: i64) -> Result<Vec<ResetInfo>> {
        let rows: Vec<ResetRow> = sqlx::query_as(
            "SELECT account_id, created_by, created_at, expires_at, used_at, used_from,
             cancelled_at IS NOT NULL AS cancelled FROM password_resets
             WHERE id IN (SELECT max(id) FROM password_resets GROUP BY account_id)
             OR used_at > ?1
             ORDER BY id DESC",
        )
        .bind(now - NOTICE_REARM_MS)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ResetInfo {
                user_id: r.account_id,
                created_by: r.created_by,
                created_at: r.created_at,
                expires_at: r.expires_at,
                used_at: r.used_at,
                used_from: r.used_from,
                cancelled: r.cancelled,
            })
            .collect())
    }

    /// Used resets of the account not yet shown to it, oldest first, and marks them shown.
    pub async fn take_reset_notices(&self, account_id: &str, now: i64) -> Result<Vec<ResetNotice>> {
        let mut tx = self.pool.begin().await?;
        let rows: Vec<(Option<String>, i64)> = sqlx::query_as(
            "SELECT b.display_name, r.used_at FROM password_resets r
             LEFT JOIN accounts b ON b.id = r.created_by
             WHERE r.account_id = ?1 AND r.used_at IS NOT NULL AND r.seen_at IS NULL
             ORDER BY r.id",
        )
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE password_resets SET seen_at = ?2
             WHERE account_id = ?1 AND used_at IS NOT NULL AND seen_at IS NULL",
        )
        .bind(account_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|(by, at)| ResetNotice { by, at })
            .collect())
    }
}
