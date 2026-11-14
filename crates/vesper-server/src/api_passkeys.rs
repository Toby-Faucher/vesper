//! Passkey routes (WebAuthn, issue #151): list, add, rename and remove your passkeys, and sign in
//! with one. A passkey sign in makes the same cookie session as a password one. Request bodies
//! hold WebAuthn responses; handlers never log them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use vesper_protocol::{
    ApiErrorCode, Me, PasskeyChallenge, PasskeyFinish, PasskeyInfo, PasskeyList, PasskeyRename,
};
use webauthn_rs::prelude::{
    DiscoverableAuthentication, Passkey, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Uuid, Webauthn, WebauthnBuilder,
};

use crate::AppState;
use crate::api_accounts::{
    ApiError, Authed, body, drop_presented_session, ip_key, limit, new_session,
};
use crate::api_admin::{bad, path};
use crate::auth::{client_ip, valid_display_name};
use crate::limits::Kind;
use vesper_store::{StoreError, now_ms};

/// A challenge lives this long, or until it is used once.
const CHALLENGE_TTL: Duration = Duration::from_secs(300);
/// Challenges held at once. Past it the oldest goes. Starting one from outside is rate limited.
const CHALLENGE_CAP: usize = 2_000;
const FAILED: &str = "passkey sign in failed";

enum Step {
    Register(String, PasskeyRegistration),
    Login(DiscoverableAuthentication),
}

/// The WebAuthn relying party and the challenges in flight. The challenges live in memory only:
/// a restart drops them and a person just starts again.
pub struct Passkeys {
    web: Option<Webauthn>,
    pending: Mutex<HashMap<String, (Instant, Step)>>,
}

impl Passkeys {
    /// The rp id is the host of `origin` and the allowed origin is `origin` itself. An origin
    /// WebAuthn cannot use (an IP address host) turns passkeys off, which Settings says.
    pub fn new(origin: &str) -> Passkeys {
        let web = Url::parse(origin).ok().and_then(|url| {
            let host = url.domain()?.to_string();
            WebauthnBuilder::new(&host, &url)
                .ok()?
                .rp_name("Vesper")
                .build()
                .ok()
        });
        if web.is_none() {
            tracing::warn!("passkeys are off: VESPER_ORIGIN has no domain name host");
        }
        Passkeys {
            web,
            pending: Mutex::default(),
        }
    }

    fn put(&self, step: Step) -> String {
        let id = vesper_store::random_hex(16);
        let now = Instant::now();
        let mut m = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        m.retain(|_, (at, _)| now.duration_since(*at) < CHALLENGE_TTL);
        if m.len() >= CHALLENGE_CAP
            && let Some(oldest) = m
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
        {
            m.remove(&oldest);
        }
        m.insert(id.clone(), (now, step));
        id
    }

    /// Removes the challenge, so it works once even if the answer is wrong.
    fn take(&self, id: &str) -> Option<Step> {
        let mut m = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, step) = m.remove(id)?;
        (at.elapsed() < CHALLENGE_TTL).then_some(step)
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/passkeys", get(list))
        .route("/passkeys/register/start", post(register_start))
        .route("/passkeys/register/finish", post(register_finish))
        .route("/passkeys/login/start", post(login_start))
        .route("/passkeys/login/finish", post(login_finish))
        .route(
            "/passkeys/{id}",
            axum::routing::patch(rename).delete(remove),
        )
}

fn off() -> ApiError {
    ApiError::new(ApiErrorCode::NotFound, "passkeys are off on this server")
}

fn web(st: &AppState) -> Result<&Webauthn, ApiError> {
    st.passkeys.web.as_ref().ok_or_else(off)
}

fn failed() -> ApiError {
    ApiError::new(ApiErrorCode::Unauthorized, FAILED)
}

async fn list(State(st): State<AppState>, a: Authed) -> Result<Json<PasskeyList>, ApiError> {
    Ok(Json(PasskeyList {
        enabled: st.passkeys.web.is_some(),
        passkeys: st.store.passkeys(&a.user.user_id).await?,
    }))
}

async fn register_start(
    State(st): State<AppState>,
    a: Authed,
) -> Result<Json<PasskeyChallenge>, ApiError> {
    let web = web(&st)?;
    limit(&st, Kind::Change, &a.user.user_id)?;
    let uuid = Uuid::parse_str(&a.user.user_id).map_err(|_| bad("bad account id"))?;
    // Already registered authenticators are told to refuse, so one key is not added twice.
    let exclude: Vec<_> = st
        .store
        .passkey_credentials(&a.user.user_id)
        .await?
        .iter()
        .filter_map(|j| {
            serde_json::from_str::<Passkey>(j)
                .inspect_err(|e| tracing::warn!("a stored passkey does not parse, skipped: {e}"))
                .ok()
        })
        .map(|p| p.cred_id().clone())
        .collect();
    let (options, state) = web
        .start_passkey_registration(
            uuid,
            &a.user.username,
            &a.user.display_name,
            (!exclude.is_empty()).then_some(exclude),
        )
        .map_err(|e| {
            tracing::error!("passkey register start: {e}");
            ApiError::new(ApiErrorCode::Internal, "server error")
        })?;
    let challenge_id = st.passkeys.put(Step::Register(a.user.user_id, state));
    Ok(Json(PasskeyChallenge {
        challenge_id,
        options: serde_json::to_value(options).map_err(|_| bad("options"))?,
    }))
}

async fn register_finish(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<PasskeyFinish>, JsonRejection>,
) -> Result<(StatusCode, Json<PasskeyInfo>), ApiError> {
    let web = web(&st)?;
    limit(&st, Kind::Change, &a.user.user_id)?;
    let req = body(req)?;
    let name = match req.name.as_deref() {
        None | Some("") => "Passkey".to_string(),
        Some(n) => valid_display_name(n)
            .ok_or_else(|| bad("name: 1 to 32 characters, no hidden characters"))?,
    };
    let invalid = || bad("this passkey could not be added, try again");
    let Some(Step::Register(owner, state)) = st.passkeys.take(&req.challenge_id) else {
        return Err(invalid());
    };
    if owner != a.user.user_id {
        return Err(invalid());
    }
    let cred: RegisterPublicKeyCredential =
        serde_json::from_value(req.credential).map_err(|_| invalid())?;
    let passkey = web
        .finish_passkey_registration(&cred, &state)
        .map_err(|e| {
            tracing::debug!("passkey register refused: {e}");
            invalid()
        })?;
    let json = serde_json::to_string(&passkey).expect("a passkey serialises");
    let now = now_ms();
    let id = st
        .store
        .add_passkey(
            &a.user.user_id,
            passkey.cred_id().as_ref(),
            &json,
            &name,
            now,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(PasskeyInfo {
            id,
            name,
            created_at: now,
            last_used_at: None,
        }),
    ))
}

async fn rename(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<PasskeyRename>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    limit(&st, Kind::Change, &a.user.user_id)?;
    let req = body(req)?;
    let name = valid_display_name(&req.name)
        .ok_or_else(|| bad("name: 1 to 32 characters, no hidden characters"))?;
    st.store.rename_passkey(&a.user.user_id, id, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    limit(&st, Kind::Change, &a.user.user_id)?;
    st.store.delete_passkey(&a.user.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Anonymous: no username is asked, so the answer says nothing about who has a passkey.
async fn login_start(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
) -> Result<Json<PasskeyChallenge>, ApiError> {
    let web = web(&st)?;
    limit(&st, Kind::LoginIp, &ip_key(client_ip(peer, &h)))?;
    let (options, state) = web.start_discoverable_authentication().map_err(|e| {
        tracing::error!("passkey login start: {e}");
        ApiError::new(ApiErrorCode::Internal, "server error")
    })?;
    let challenge_id = st.passkeys.put(Step::Login(state));
    Ok(Json(PasskeyChallenge {
        challenge_id,
        options: serde_json::to_value(options).map_err(|_| bad("options"))?,
    }))
}

/// Every failure is the same 401. A banned account, an unknown or removed passkey, a wrong
/// origin, a replayed or expired challenge and a bad signature all look alike.
async fn login_finish(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    req: Result<Json<PasskeyFinish>, JsonRejection>,
) -> Result<Response, ApiError> {
    let web = web(&st)?;
    limit(&st, Kind::LoginIp, &ip_key(client_ip(peer, &h)))?;
    let req = body(req)?;
    let Some(Step::Login(state)) = st.passkeys.take(&req.challenge_id) else {
        return Err(failed());
    };
    let cred: PublicKeyCredential = serde_json::from_value(req.credential).map_err(|_| failed())?;
    let (uuid, cred_id) = web
        .identify_discoverable_authentication(&cred)
        .map_err(|_| failed())?;
    let row = st
        .store
        .passkey_by_credential(cred_id)
        .await?
        .filter(|r| !r.banned && r.account_id == uuid.simple().to_string())
        .ok_or_else(failed)?;
    let mut passkey: Passkey = serde_json::from_str(&row.credential).map_err(|_| failed())?;
    let result = web
        .finish_discoverable_authentication_v2(&cred, state, &passkey)
        .map_err(|e| {
            tracing::debug!("passkey sign in refused: {e}");
            failed()
        })?;
    if passkey.update_credential(&result).is_some() {
        let json = serde_json::to_string(&passkey).expect("a passkey serialises");
        st.store.passkey_used(row.id, &json, now_ms()).await?;
    } else {
        st.store
            .passkey_used(row.id, &row.credential, now_ms())
            .await?;
    }
    drop_presented_session(&h, &st).await?;
    let (session_id, cookie) = new_session(&st, &row.account_id, &h).await?;
    let user = st.store.user(&row.account_id).await.map_err(|e| match e {
        StoreError::NotFound => failed(),
        e => e.into(),
    })?;
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(Me {
            user,
            session_id,
            server: st.cfg.server_build(),
            upload_max: st.cfg.upload_max_bytes as i64,
        }),
    )
        .into_response())
}
