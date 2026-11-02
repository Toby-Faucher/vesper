//! Push routes: the VAPID public key and this account's push devices, this browser's
//! subscription (set and remove), removing another device, and a test push to this browser.
//! Endpoints and keys are never logged or sent back.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use vesper_protocol::{ApiErrorCode, PushInfo, PushPayload, PushSubscribe};
use vesper_store::{NewPushSub, StoreError, TOO_MANY_DEVICES, now_ms};

use crate::api_accounts::{body, limit};
use crate::api_admin::{bad, path};
use crate::auth::device_label;
use crate::limits::Kind;
use crate::push::{Sent, encrypt, parse_endpoint};
use crate::{ApiError, AppState, Authed};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/push", get(info))
        .route("/push/subscription", put(subscribe).delete(unsubscribe))
        .route("/push/devices/{id}", delete(remove_device))
        .route("/push/test", post(test))
}

fn off() -> ApiError {
    ApiError::new(ApiErrorCode::NotFound, "push is off on this server")
}

async fn info(State(st): State<AppState>, a: Authed) -> Result<Json<PushInfo>, ApiError> {
    Ok(Json(PushInfo {
        public_key: st.push.public_key().map(str::to_string),
        devices: st.store.push_devices(&a.user.user_id, a.session_id).await?,
    }))
}

async fn subscribe(
    State(st): State<AppState>,
    a: Authed,
    h: HeaderMap,
    req: Result<Json<PushSubscribe>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    st.push.public_key().ok_or_else(off)?;
    limit(&st, Kind::Push, &a.user.user_id)?;
    let req = body(req)?;
    parse_endpoint(&req.endpoint, st.push.allow_loopback()).map_err(bad)?;
    let p256dh = B64.decode(req.p256dh.trim_end_matches('=')).ok();
    let auth = B64.decode(req.auth.trim_end_matches('=')).ok();
    let (Some(p256dh), Some(auth)) = (p256dh, auth) else {
        return Err(bad("push keys are not base64url"));
    };
    // A trial encryption checks the point is on the curve and the sizes are right.
    encrypt(b"", &p256dh, &auth).map_err(|_| bad("push keys are not valid"))?;
    let label = device_label(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()));
    st.store
        .put_push_subscription(NewPushSub {
            account_id: &a.user.user_id,
            session_id: a.session_id,
            endpoint: &req.endpoint,
            p256dh: &p256dh,
            auth: &auth,
            mentions: req.mentions,
            voice: req.voice,
            user_agent_hint: &label,
            now: now_ms(),
        })
        .await
        .map_err(|e| match e {
            StoreError::Conflict(TOO_MANY_DEVICES) => ApiError::new(
                ApiErrorCode::Conflict,
                "at most 5 devices get notifications: remove one first",
            ),
            // Plain refusal: the endpoint is a capability URL the caller already holds, so
            // this tells them nothing new, and pretending success would hide a real mix-up.
            StoreError::Conflict(_) => ApiError::new(
                ApiErrorCode::Conflict,
                "this browser gets notifications for another session: turn them off there first",
            ),
            e => e.into(),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unsubscribe(State(st): State<AppState>, a: Authed) -> Result<StatusCode, ApiError> {
    st.store.delete_push_of_session(a.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_device(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    st.store
        .delete_push_device(&a.user.user_id, path(id)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends to this browser and waits for the push service's answer.
async fn test(State(st): State<AppState>, a: Authed) -> Result<StatusCode, ApiError> {
    st.push.public_key().ok_or_else(off)?;
    limit(&st, Kind::Push, &a.user.user_id)?;
    let t = st
        .store
        .push_target_of_session(a.session_id)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                ApiErrorCode::NotFound,
                "this browser has no push subscription",
            )
        })?;
    let payload = PushPayload {
        kind: "test".into(),
        room_id: 0,
        title: "Vesper".into(),
        body: "Notifications reach this device.".into(),
    };
    let body = serde_json::to_vec(&payload).expect("payload serialises");
    match st.push.send(&t, &body, 60, "high", "test").await {
        Sent::Ok => Ok(StatusCode::NO_CONTENT),
        Sent::Gone => Err(ApiError::new(
            ApiErrorCode::NotFound,
            "the push service no longer knows this browser: turn notifications off and on",
        )),
        Sent::Failed(_) => Err(ApiError::new(
            ApiErrorCode::Internal,
            "the push service did not take the notification",
        )),
    }
}
