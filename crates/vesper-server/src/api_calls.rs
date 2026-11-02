//! Your recent calls: `POST /api/me/calls` stores one summary of a call you left, sent by your
//! own client; `GET /api/me/calls` returns your last 20. Nobody reads another account's calls,
//! the owner included. Rows older than 30 days are deleted at start and once a day.

use std::time::Duration;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use vesper_protocol::{CallRecord, CallSummary};
use vesper_store::{CALLS_TTL_MS, now_ms};

use crate::api_accounts::{body, limit};
use crate::api_admin::bad;
use crate::limits::Kind;
use crate::{ApiError, AppState, Authed};

pub const MAX_DURATION_S: u32 = 24 * 3600;
pub const MAX_REASON_CHARS: usize = 120;
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/calls", get(list).post(add))
}

async fn list(State(st): State<AppState>, a: Authed) -> Result<Json<Vec<CallRecord>>, ApiError> {
    Ok(Json(st.store.calls_of(&a.user.user_id).await?))
}

async fn add(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<CallSummary>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    limit(&st, Kind::Calls, &a.user.user_id)?;
    let s = body(req)?;
    check(&s).map_err(bad)?;
    // Always the caller's account: the body has no account field, and an extra one is ignored.
    st.store.add_call(&a.user.user_id, &s, now_ms()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every figure finite and in range, the reason short and printable.
pub fn check(s: &CallSummary) -> Result<(), &'static str> {
    let within =
        |v: Option<f64>, hi: f64| v.is_none_or(|v| v.is_finite() && (0.0..=hi).contains(&v));
    if s.duration_s > MAX_DURATION_S {
        return Err("duration_s is over 24 hours");
    }
    if s.fallback_reason.chars().count() > MAX_REASON_CHARS
        || s.fallback_reason.chars().any(char::is_control)
    {
        return Err("fallback_reason is too long or has control characters");
    }
    if !within(s.loss_pct, 100.0) || !within(s.concealed_pct, 100.0) {
        return Err("a percentage is out of range");
    }
    if !within(s.jitter_ms, 10_000.0) || !within(s.rtt_ms, 60_000.0) {
        return Err("a time is out of range");
    }
    Ok(())
}

/// Deletes calls older than 30 days now and then once a day.
pub fn spawn_prune(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PRUNE_EVERY);
        loop {
            tick.tick().await;
            match st.store.prune_calls(now_ms() - CALLS_TTL_MS).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("pruned {n} call summaries older than 30 days"),
                Err(e) => tracing::error!("call summary prune: {e}"),
            }
        }
    });
}
