//! Web Push rows: server secrets (the VAPID key) and one push subscription per session.

use sqlx::FromRow;
use vesper_protocol::PushDevice;

use crate::sessions::SESSION_TTL_MS;
use crate::{Conflict, Result, Store, StoreError, constraint};

/// Devices (sessions) per account that get push. Tabs of one browser share a session, so one row.
pub const MAX_PUSH_DEVICES: i64 = 5;

/// Which switch a push needs. The test push goes to one session whatever its switches say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    Mentions,
    Voice,
}

pub struct NewPushSub<'a> {
    pub account_id: &'a str,
    pub session_id: i64,
    pub endpoint: &'a str,
    pub p256dh: &'a [u8],
    pub auth: &'a [u8],
    pub mentions: bool,
    pub voice: bool,
    pub user_agent_hint: &'a str,
    pub now: i64,
}

/// Where to send one push. The endpoint and keys never leave the server except to the endpoint.
#[derive(Clone, PartialEq, Eq, FromRow)]
pub struct PushTarget {
    pub id: i64,
    pub account_id: String,
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>,
}

impl std::fmt::Debug for PushTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushTarget")
            .field("id", &self.id)
            .field("account_id", &self.account_id)
            .field("endpoint", &"***")
            .finish()
    }
}

#[derive(FromRow)]
struct DeviceRow {
    id: i64,
    current: bool,
    label: String,
    created_at: i64,
    last_ok_at: Option<i64>,
    mentions: bool,
    voice: bool,
}

impl Store {
    /// The secret called `name`, made by `make` and stored on first use. Two callers at once
    /// both get the one that was stored first.
    pub async fn secret_or_insert(
        &self,
        name: &str,
        make: impl FnOnce() -> Vec<u8>,
        now: i64,
    ) -> Result<Vec<u8>> {
        sqlx::query(
            "INSERT INTO server_secrets (name, value, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO NOTHING",
        )
        .bind(name)
        .bind(make())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(
            sqlx::query_scalar("SELECT value FROM server_secrets WHERE name = ?1")
                .bind(name)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Sets this session's subscription, replacing its earlier one. A row of another session
    /// with the same endpoint is taken over only when the keys match too (the same browser,
    /// now signed in as someone else): knowing an endpoint is not enough to take it.
    /// `Conflict(EndpointTaken)` otherwise, `Conflict(Conflict::TooManyDevices)` past [`MAX_PUSH_DEVICES`].
    pub async fn put_push_subscription(&self, s: NewPushSub<'_>) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM push_subscriptions WHERE session_id = ?1
               OR (endpoint = ?2 AND p256dh = ?3 AND auth = ?4)",
        )
        .bind(s.session_id)
        .bind(s.endpoint)
        .bind(s.p256dh)
        .bind(s.auth)
        .execute(&mut *tx)
        .await?;
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE account_id = ?1")
                .bind(s.account_id)
                .fetch_one(&mut *tx)
                .await?;
        if n >= MAX_PUSH_DEVICES {
            return Err(StoreError::Conflict(Conflict::TooManyDevices));
        }
        let id = sqlx::query_scalar(
            "INSERT INTO push_subscriptions (account_id, session_id, endpoint, p256dh, auth,
                 mentions, voice, created_at, user_agent_hint)
             SELECT ?1, id, ?3, ?4, ?5, ?6, ?7, ?8, ?9 FROM sessions
             WHERE id = ?2 AND account_id = ?1 RETURNING id",
        )
        .bind(s.account_id)
        .bind(s.session_id)
        .bind(s.endpoint)
        .bind(s.p256dh)
        .bind(s.auth)
        .bind(s.mentions)
        .bind(s.voice)
        .bind(s.now)
        .bind(s.user_agent_hint)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| constraint(e, Conflict::EndpointTaken))?
        .ok_or(StoreError::NotFound)?;
        tx.commit().await?;
        Ok(id)
    }

    /// Subscriptions of `accounts` with `kind` on, whose session is live and account not banned.
    pub async fn push_targets(
        &self,
        accounts: &[String],
        kind: PushKind,
        now: i64,
    ) -> Result<Vec<PushTarget>> {
        if accounts.is_empty() {
            return Ok(Vec::new());
        }
        let voice = kind == PushKind::Voice;
        let ids = serde_json::to_string(accounts).expect("strings serialise");
        Ok(sqlx::query_as(
            "SELECT p.id, p.account_id, p.endpoint, p.p256dh, p.auth FROM push_subscriptions p
             JOIN sessions s ON s.id = p.session_id JOIN accounts a ON a.id = p.account_id
             WHERE p.account_id IN (SELECT value FROM json_each(?1))
               AND (CASE WHEN ?3 THEN p.voice ELSE p.mentions END) = 1
               AND a.banned_at IS NULL AND s.last_seen_at >= ?2
             ORDER BY p.id",
        )
        .bind(ids)
        .bind(now - SESSION_TTL_MS)
        .bind(voice)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The subscription of one session, for the test push.
    pub async fn push_target_of_session(&self, session_id: i64) -> Result<Option<PushTarget>> {
        Ok(sqlx::query_as(
            "SELECT id, account_id, endpoint, p256dh, auth FROM push_subscriptions
             WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// This account's push devices, newest first; `current` marks the given session's one.
    pub async fn push_devices(&self, account_id: &str, current: i64) -> Result<Vec<PushDevice>> {
        let rows: Vec<DeviceRow> = sqlx::query_as(
            "SELECT id, session_id = ?2 AS current, user_agent_hint AS label, created_at,
                 last_ok_at, mentions, voice
             FROM push_subscriptions WHERE account_id = ?1 ORDER BY created_at DESC, id DESC",
        )
        .bind(account_id)
        .bind(current)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| PushDevice {
                id: r.id,
                label: r.label,
                created_at: r.created_at,
                last_ok_at: r.last_ok_at,
                current: r.current,
                mentions: r.mentions,
                voice: r.voice,
            })
            .collect())
    }

    /// `NotFound` unless subscription `id` belongs to `account_id`.
    pub async fn delete_push_device(&self, account_id: &str, id: i64) -> Result<()> {
        let n = sqlx::query("DELETE FROM push_subscriptions WHERE id = ?1 AND account_id = ?2")
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

    /// This session's subscription, if any (push turned off on this device).
    pub async fn delete_push_of_session(&self, session_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM push_subscriptions WHERE session_id = ?1")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The push service said the subscription is gone (404 or 410).
    pub async fn delete_push_subscription(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM push_subscriptions WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn push_delivered(&self, id: i64, now: i64) -> Result<()> {
        sqlx::query("UPDATE push_subscriptions SET last_ok_at = ?2 WHERE id = ?1")
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
