//! Account routes: invite preview, signup, login, logout, profile, password and devices.
//! Handlers never log request bodies: they hold passwords, tokens and invite codes.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{ConnectInfo, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use vesper_protocol::{
    ApiErrorCode, CLOSE_SESSION_ENDED, Device, InvitePreview, LoginReq, Me, PasswordChange,
    ProfilePatch, Role, SignupReq, UserInfo,
};
use vesper_store::{NewAccount, NewSession, StoreError, now_ms};

use crate::AppState;
use crate::auth::{
    clear_cookie, client_ip, cookie_token, device_label, hash_password, hash_token, new_token,
    session_cookie, valid_accent, valid_display_name, valid_password, valid_username,
    verify_password,
};
use crate::limits::Kind;

const WRONG_LOGIN: &str = "wrong username or password";
const BAD_INVITE: &str = "invite not valid";

/// A failed `/api` call: the wire body plus, for 429, how long to wait.
#[derive(Debug)]
pub struct ApiError {
    pub body: vesper_protocol::ApiError,
    pub retry_after: Option<Duration>,
}

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> ApiError {
        ApiError {
            body: vesper_protocol::ApiError {
                code,
                message: message.into(),
            },
            retry_after: None,
        }
    }

    fn rate_limited(wait: Duration) -> ApiError {
        ApiError {
            retry_after: Some(wait),
            ..ApiError::new(
                ApiErrorCode::RateLimited,
                "too many attempts, try again later",
            )
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.body.code {
            ApiErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ApiErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ApiErrorCode::NotFound | ApiErrorCode::InviteInvalid => StatusCode::NOT_FOUND,
            ApiErrorCode::Conflict => StatusCode::CONFLICT,
            ApiErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ApiErrorCode::QuotaFull => StatusCode::INSUFFICIENT_STORAGE,
            ApiErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let mut res = (status, Json(self.body)).into_response();
        if let Some(wait) = self.retry_after {
            let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs.max(1)));
        }
        res
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> ApiError {
        match e {
            StoreError::NotFound => ApiError::new(ApiErrorCode::NotFound, "not found"),
            StoreError::Conflict(what) => ApiError::new(ApiErrorCode::Conflict, what),
            StoreError::InviteInvalid => ApiError::new(ApiErrorCode::InviteInvalid, BAD_INVITE),
            StoreError::Db(e) => {
                tracing::error!("store: {e}");
                ApiError::new(ApiErrorCode::Internal, "server error")
            }
        }
    }
}

fn bad(message: &str) -> ApiError {
    ApiError::new(ApiErrorCode::BadRequest, message)
}

fn unauthorized() -> ApiError {
    ApiError::new(ApiErrorCode::Unauthorized, "not signed in")
}

/// Unwraps a JSON body; the 32 KiB cap shows as 413, anything else as 400.
pub fn body<T>(r: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    match r {
        Ok(Json(t)) => Ok(t),
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => Err(ApiError::new(
            ApiErrorCode::TooLarge,
            "request body too large",
        )),
        // Fixed text: the rejection's own text holds serde internals.
        Err(_) => Err(bad("invalid request body")),
    }
}

/// The signed-in user of the request's session cookie.
pub struct Authed {
    pub user: UserInfo,
    pub session_id: i64,
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Authed, ApiError> {
        let (session_id, user) = session_of(&parts.headers, st)
            .await?
            .ok_or_else(unauthorized)?;
        Ok(Authed { user, session_id })
    }
}

/// The live session named by the cookie, if any.
pub(crate) async fn session_of(
    h: &HeaderMap,
    st: &AppState,
) -> Result<Option<(i64, UserInfo)>, ApiError> {
    let Some(hash) = cookie_token(h, st.cfg.secure_cookies).and_then(|t| hash_token(&t)) else {
        return Ok(None);
    };
    Ok(st.store.session_user(&hash, now_ms()).await?)
}

/// Session fixation: a cookie sent with a login or signup never survives it. Already gone
/// (another tab logged out meanwhile) is fine.
async fn drop_presented_session(h: &HeaderMap, st: &AppState) -> Result<(), ApiError> {
    if let Some((sid, old)) = session_of(h, st).await? {
        st.hub.close_session(sid, CLOSE_SESSION_ENDED);
        match st.store.delete_session(&old.user_id, sid).await {
            Ok(()) | Err(StoreError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Rate-limit key for an address: IPv6 clients are keyed on their /64, as one host usually
/// holds the whole prefix.
fn ip_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let net = std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0);
            format!("{net}/64")
        }
    }
}

fn limit(st: &AppState, kind: Kind, key: &str) -> Result<(), ApiError> {
    st.limits
        .check(kind, key, Instant::now())
        .map_err(ApiError::rate_limited)
}

/// A fresh session for `user_id`: the token goes only into the returned cookie.
async fn new_session(
    st: &AppState,
    user_id: &str,
    h: &HeaderMap,
) -> Result<(i64, HeaderValue), ApiError> {
    let (token, token_hash) = new_token();
    let label = device_label(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()));
    let s = NewSession {
        token_hash,
        label: &label,
        now: now_ms(),
    };
    let sid = st.store.create_session(user_id, s).await?;
    Ok((sid, session_cookie(&token, st.cfg.secure_cookies)))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/invites/preview", get(preview))
        .route("/signup", post(signup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me).patch(patch_me))
        .route("/me/password", post(change_password))
        .route("/sessions", get(sessions))
        .route("/sessions/{id}", delete(revoke))
}

#[derive(Deserialize)]
struct PreviewQuery {
    code: String,
}

/// The role an invite grants. Unknown, used, expired, revoked and spent owner invites all look
/// the same.
async fn preview(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    q: Result<Query<PreviewQuery>, QueryRejection>,
) -> Result<Json<InvitePreview>, ApiError> {
    limit(&st, Kind::Invite, &ip_key(client_ip(peer, &h)))?;
    let q = q.map_err(|_| bad("invalid query"))?;
    let invalid = || ApiError::new(ApiErrorCode::InviteInvalid, BAD_INVITE);
    let hash = hash_token(&q.code).ok_or_else(invalid)?;
    let role = st
        .store
        .invite_role(&hash, now_ms())
        .await?
        .ok_or_else(invalid)?;
    if role == Role::Owner && st.store.owner_exists().await? {
        return Err(invalid());
    }
    Ok(Json(InvitePreview { role }))
}

async fn signup(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    req: Result<Json<SignupReq>, JsonRejection>,
) -> Result<Response, ApiError> {
    limit(&st, Kind::Invite, &ip_key(client_ip(peer, &h)))?;
    let req = body(req)?;
    let username =
        valid_username(&req.username).ok_or_else(|| bad("username: 3 to 24 of a-z, 0-9 and _"))?;
    let display_name = valid_display_name(&req.display_name)
        .ok_or_else(|| bad("display name: 1 to 32 characters, no hidden characters"))?;
    if !valid_password(&req.password) {
        return Err(bad("password: 8 to 128 characters"));
    }
    let invite_hash = hash_token(&req.invite)
        .ok_or_else(|| ApiError::new(ApiErrorCode::InviteInvalid, BAD_INVITE))?;
    // Cheap check first so a junk code costs no argon2 hash; signup checks again in its transaction.
    if st
        .store
        .invite_role(&invite_hash, now_ms())
        .await?
        .is_none()
    {
        return Err(ApiError::new(ApiErrorCode::InviteInvalid, BAD_INVITE));
    }
    let phc = hash_password(req.password).await;
    let (token, token_hash) = new_token();
    let label = device_label(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()));
    let account = NewAccount {
        username: &username,
        display_name: &display_name,
        password_phc: Some(&phc),
        role: Role::Member,
    };
    let s = NewSession {
        token_hash,
        label: &label,
        now: now_ms(),
    };
    let (user, session_id) = st.store.signup(&invite_hash, account, s).await?;
    drop_presented_session(&h, &st).await?;
    let cookie = session_cookie(&token, st.cfg.secure_cookies);
    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(Me { user, session_id }),
    )
        .into_response())
}

/// Every failure is the same 401. An unknown user still costs one argon2 verify, so timing should
/// not tell whether the name exists (reasoned, not measured). A name that fails the format check
/// is refused before argon2.
async fn login(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    req: Result<Json<LoginReq>, JsonRejection>,
) -> Result<Response, ApiError> {
    let ip = ip_key(client_ip(peer, &h));
    limit(&st, Kind::LoginIp, &ip)?;
    let req = body(req)?;
    let wrong = || ApiError::new(ApiErrorCode::Unauthorized, WRONG_LOGIN);
    // Validated before it becomes part of a limiter key, so keys stay short.
    let username = valid_username(&req.username).ok_or_else(wrong)?;
    limit(&st, Kind::Login, &format!("{ip} {username}"))?;
    let row = st.store.login_row(&username).await?;
    let phc = row.as_ref().and_then(|r| r.password_phc.clone());
    let ok = verify_password(req.password, phc).await;
    let Some(row) = row.filter(|r| ok && !r.banned) else {
        return Err(wrong());
    };
    drop_presented_session(&h, &st).await?;
    let (session_id, cookie) = new_session(&st, &row.user_id, &h).await?;
    let user = st.store.user(&row.user_id).await?;
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(Me { user, session_id }),
    )
        .into_response())
}

async fn logout(State(st): State<AppState>, a: Authed) -> Result<Response, ApiError> {
    st.store
        .delete_session(&a.user.user_id, a.session_id)
        .await?;
    st.hub.close_session(a.session_id, CLOSE_SESSION_ENDED);
    let cookie = clear_cookie(st.cfg.secure_cookies);
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

async fn me(a: Authed) -> Json<Me> {
    Json(Me {
        user: a.user,
        session_id: a.session_id,
    })
}

async fn patch_me(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<ProfilePatch>, JsonRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    let req = body(req)?;
    let display_name = match req.display_name {
        Some(n) => Some(
            valid_display_name(&n)
                .ok_or_else(|| bad("display name: 1 to 32 characters, no hidden characters"))?,
        ),
        None => None,
    };
    let accent = match req.accent {
        Some(c) => Some(valid_accent(&c).ok_or_else(|| bad("accent: a colour like #7c8cff"))?),
        None => None,
    };
    let user = st
        .store
        .update_profile(&a.user.user_id, display_name.as_deref(), accent.as_deref())
        .await?;
    st.hub.update_user(user.clone());
    Ok(Json(user))
}

/// Checks the old password (rate-limited like a login), then drops every other session.
async fn change_password(
    State(st): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    a: Authed,
    req: Result<Json<PasswordChange>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let req = body(req)?;
    if !valid_password(&req.new) {
        return Err(bad("password: 8 to 128 characters"));
    }
    let ip = ip_key(client_ip(peer, &h));
    limit(&st, Kind::Login, &format!("{ip} {}", a.user.username))?;
    let phc = st
        .store
        .login_row(&a.user.username)
        .await?
        .and_then(|r| r.password_phc);
    // 403, not 401: the session is still good, only the old password is wrong.
    if !verify_password(req.old, phc).await {
        return Err(ApiError::new(ApiErrorCode::Forbidden, "wrong password"));
    }
    let phc = hash_password(req.new).await;
    st.store
        .set_password(&a.user.user_id, &phc, a.session_id)
        .await?;
    st.hub
        .close_user_except(&a.user.user_id, Some(a.session_id), CLOSE_SESSION_ENDED);
    Ok(StatusCode::NO_CONTENT)
}

async fn sessions(State(st): State<AppState>, a: Authed) -> Result<Json<Vec<Device>>, ApiError> {
    Ok(Json(
        st.store.sessions(&a.user.user_id, a.session_id).await?,
    ))
}

/// Revoking the current session also clears its cookie, like logout.
async fn revoke(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<Response, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::new(ApiErrorCode::NotFound, "not found"))?;
    st.store.delete_session(&a.user.user_id, id).await?;
    st.hub.close_session(id, CLOSE_SESSION_ENDED);
    if id == a.session_id {
        let cookie = clear_cookie(st.cfg.secure_cookies);
        return Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_is_keyed_on_the_64() {
        let k = |s: &str| ip_key(s.parse().unwrap());
        assert_eq!(k("1.2.3.4"), "1.2.3.4");
        assert_eq!(k("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
        assert_eq!(k("2001:db8:1:2:bbbb::9"), k("2001:db8:1:2:aaaa::1"));
        assert_ne!(k("2001:db8:1:3::1"), k("2001:db8:1:2::1"));
    }
}
