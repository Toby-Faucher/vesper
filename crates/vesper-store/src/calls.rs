//! Your recent calls: one summary row per call, readable only by the account that sent it.

use sqlx::FromRow;
use vesper_protocol::{CallMode, CallRecord, CallSummary};

use crate::{Result, Store, StoreError};

/// Rows kept per account: the list shows this many, so older ones are never read.
pub const CALLS_KEPT: i64 = 20;
/// Rows older than this are deleted (at start and once a day).
pub const CALLS_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(FromRow)]
struct Row {
    id: i64,
    started_at: i64,
    ended_at: i64,
    mode: String,
    fallback_reason: String,
    loss_pct: Option<f64>,
    jitter_ms: Option<f64>,
    concealed_pct: Option<f64>,
    rtt_ms: Option<f64>,
    browser_ns_loading: bool,
}

fn mode_str(m: CallMode) -> &'static str {
    match m {
        CallMode::Deepfilter => "deepfilter",
        CallMode::Browser => "browser",
        CallMode::Off => "off",
    }
}

impl TryFrom<Row> for CallRecord {
    type Error = StoreError;
    fn try_from(r: Row) -> Result<CallRecord> {
        let mode = match r.mode.as_str() {
            "deepfilter" => CallMode::Deepfilter,
            "browser" => CallMode::Browser,
            "off" => CallMode::Off,
            m => {
                return Err(StoreError::Db(sqlx::Error::Decode(
                    format!("unknown call mode {m:?}").into(),
                )));
            }
        };
        Ok(CallRecord {
            id: r.id,
            started_at: r.started_at,
            ended_at: r.ended_at,
            summary: CallSummary {
                duration_s: ((r.ended_at - r.started_at) / 1000).clamp(0, u32::MAX as i64) as u32,
                mode,
                fallback_reason: r.fallback_reason,
                loss_pct: r.loss_pct,
                jitter_ms: r.jitter_ms,
                concealed_pct: r.concealed_pct,
                rtt_ms: r.rtt_ms,
                browser_ns_loading: r.browser_ns_loading,
            },
        })
    }
}

impl Store {
    /// Stores `s` for `account_id`, ending at `now`, and drops that account's rows past [`CALLS_KEPT`].
    /// The caller has checked the ranges; the table's CHECKs refuse anything else.
    pub async fn add_call(&self, account_id: &str, s: &CallSummary, now: i64) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let id = sqlx::query_scalar(
            "INSERT INTO call_stats (account_id, started_at, ended_at, mode, fallback_reason,
                 loss_pct, jitter_ms, concealed_pct, rtt_ms, browser_ns_loading)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) RETURNING id",
        )
        .bind(account_id)
        .bind(now - i64::from(s.duration_s) * 1000)
        .bind(now)
        .bind(mode_str(s.mode))
        .bind(&s.fallback_reason)
        .bind(s.loss_pct)
        .bind(s.jitter_ms)
        .bind(s.concealed_pct)
        .bind(s.rtt_ms)
        .bind(s.browser_ns_loading)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM call_stats WHERE account_id = ?1 AND id NOT IN
                 (SELECT id FROM call_stats WHERE account_id = ?1 ORDER BY id DESC LIMIT ?2)",
        )
        .bind(account_id)
        .bind(CALLS_KEPT)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// The newest [`CALLS_KEPT`] calls of `account_id`, newest first. Nobody else's.
    pub async fn calls_of(&self, account_id: &str) -> Result<Vec<CallRecord>> {
        sqlx::query_as::<_, Row>(
            "SELECT id, started_at, ended_at, mode, fallback_reason, loss_pct, jitter_ms,
                 concealed_pct, rtt_ms, browser_ns_loading
             FROM call_stats WHERE account_id = ?1 ORDER BY id DESC LIMIT ?2",
        )
        .bind(account_id)
        .bind(CALLS_KEPT)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(CallRecord::try_from)
        .collect()
    }

    /// Deletes calls that ended before `before`. Returns how many.
    pub async fn prune_calls(&self, before: i64) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM call_stats WHERE ended_at < ?1")
            .bind(before)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }
}
