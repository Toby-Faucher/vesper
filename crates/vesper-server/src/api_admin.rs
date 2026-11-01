//! Admin routes: the user list, invites, roles, bans and the voice actions.
//! Owner > moderator > member. Moderators mint and revoke member invites; only the owner mints
//! moderator invites, changes roles and bans. The owner role only comes from the CLI, so it can
//! never be granted, taken away or banned here. Kick from voice, force-mute and disconnect need
//! moderator, and only work on a lower role (a moderator on members, the owner on anyone else).

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use vesper_protocol::{
    ApiErrorCode, CLOSE_DISCONNECTED, CLOSE_SESSION_ENDED, ForceMute, InviteCreate, InviteCreated,
    InviteInfo, Role, RolePatch, UserInfo,
};
use vesper_store::now_ms;

use crate::api_accounts::body;
use crate::auth::new_token;
use crate::{ApiError, AppState, Authed};

const HOUR_MS: i64 = 3_600_000;

fn rank(r: Role) -> u8 {
    match r {
        Role::Member => 0,
        Role::Moderator => 1,
        Role::Owner => 2,
    }
}

/// 403 unless `user` holds `min` or a higher role.
pub fn require(user: &UserInfo, min: Role) -> Result<(), ApiError> {
    if rank(user.role) >= rank(min) {
        Ok(())
    } else {
        Err(ApiError::new(ApiErrorCode::Forbidden, "not allowed"))
    }
}

pub(crate) fn bad(message: &str) -> ApiError {
    ApiError::new(ApiErrorCode::BadRequest, message)
}

pub(crate) fn path<T>(p: Result<Path<T>, PathRejection>) -> Result<T, ApiError> {
    p.map(|Path(t)| t)
        .map_err(|_| ApiError::new(ApiErrorCode::NotFound, "not found"))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(users))
        .route("/invites", get(invites).post(create_invite))
        .route("/invites/{id}", delete(revoke_invite))
        .route("/users/{id}/role", patch(set_role))
        .route("/users/{id}/ban", post(ban))
        .route("/users/{id}/unban", post(unban))
        .route("/users/{id}/kick", post(kick))
        .route("/users/{id}/force-mute", post(force_mute))
        .route("/users/{id}/disconnect", post(disconnect))
}

async fn users(State(st): State<AppState>, _a: Authed) -> Result<Json<Vec<UserInfo>>, ApiError> {
    Ok(Json(st.store.users().await?))
}

async fn invites(State(st): State<AppState>, a: Authed) -> Result<Json<Vec<InviteInfo>>, ApiError> {
    require(&a.user, Role::Moderator)?;
    Ok(Json(st.store.invites().await?))
}

async fn create_invite(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<InviteCreate>, JsonRejection>,
) -> Result<(StatusCode, Json<InviteCreated>), ApiError> {
    require(&a.user, Role::Moderator)?;
    let req = body(req)?;
    if req.role == Role::Owner {
        return Err(bad("the owner role only comes from the command line"));
    }
    if !(1..=50).contains(&req.max_uses) {
        return Err(bad("max_uses: 1 to 50"));
    }
    if !(1..=720).contains(&req.expires_hours) {
        return Err(bad("expires_hours: 1 to 720"));
    }
    // Moderators mint member invites only; nobody mints a role above their own.
    if req.role != Role::Member {
        require(&a.user, Role::Owner)?;
    }
    let (code, hash) = new_token();
    let expires_at = now_ms() + i64::from(req.expires_hours) * HOUR_MS;
    let info = st
        .store
        .create_invite(
            &hash,
            req.role,
            req.max_uses,
            expires_at,
            Some(&a.user.user_id),
        )
        .await?;
    let url = format!("{}/invite#{code}", st.cfg.origin);
    Ok((StatusCode::CREATED, Json(InviteCreated { info, url })))
}

async fn revoke_invite(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    require(&a.user, Role::Moderator)?;
    let id = path(id)?;
    // Moderators revoke member invites only. An unknown id falls through to the store's 404.
    if a.user.role != Role::Owner {
        let invites = st.store.invites().await?;
        if invites.iter().any(|i| i.id == id && i.role != Role::Member) {
            return Err(ApiError::new(ApiErrorCode::Forbidden, "not allowed"));
        }
    }
    st.store.revoke_invite(id, now_ms()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Owner only, and never on themselves or another owner, so the owner can not lock everyone out.
async fn owner_target(st: &AppState, a: &Authed, id: &str) -> Result<(), ApiError> {
    require(&a.user, Role::Owner)?;
    if id == a.user.user_id {
        return Err(bad("you can not change your own account here"));
    }
    if st.store.user(id).await?.role == Role::Owner {
        return Err(bad("the owner can not be changed here"));
    }
    Ok(())
}

async fn set_role(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
    req: Result<Json<RolePatch>, JsonRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    let id = path(id)?;
    owner_target(&st, &a, &id).await?;
    let req = body(req)?;
    if req.role == Role::Owner {
        return Err(bad("the owner role only comes from the command line"));
    }
    let user = st.store.set_role(&id, req.role).await?;
    st.hub.update_user(user.clone());
    // The role decides which restricted rooms they see.
    crate::api_rooms::rooms_changed(&st).await;
    Ok(Json(user))
}

/// The store deletes every session of the user in the same transaction.
async fn ban(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    let id = path(id)?;
    owner_target(&st, &a, &id).await?;
    let user = st.store.set_banned(&id, true, now_ms()).await?;
    after_ban(&st, &user);
    Ok(Json(user))
}

async fn unban(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    let id = path(id)?;
    owner_target(&st, &a, &id).await?;
    let user = st.store.set_banned(&id, false, now_ms()).await?;
    st.hub.update_user(user.clone());
    Ok(Json(user))
}

/// Moderator or owner, acting on a strictly lower role (so never on themselves, the owner or a
/// fellow moderator). The role is read fresh on every call by `Authed`. Unknown id: 404.
async fn voice_target(st: &AppState, a: &Authed, id: &str) -> Result<(), ApiError> {
    require(&a.user, Role::Moderator)?;
    if rank(st.store.user(id).await?.role) >= rank(a.user.role) {
        return Err(ApiError::new(ApiErrorCode::Forbidden, "not allowed"));
    }
    Ok(())
}

/// Take the user out of voice. 404 if they are not in voice.
async fn kick(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    voice_target(&st, &a, &id).await?;
    if !st.voice.kick(&id) {
        return Err(ApiError::new(ApiErrorCode::NotFound, "not in voice"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Works whether or not the user is in voice now: it holds until lifted.
async fn force_mute(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
    req: Result<Json<ForceMute>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    voice_target(&st, &a, &id).await?;
    st.voice.force_mute(&id, body(req)?.muted);
    Ok(StatusCode::NO_CONTENT)
}

/// Close every socket of the user with 4003. The sessions stay valid, so they can reconnect;
/// a ban is the way to keep someone out.
async fn disconnect(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    voice_target(&st, &a, &id).await?;
    st.voice.kick(&id);
    st.hub.close_user(&id, CLOSE_DISCONNECTED);
    Ok(StatusCode::NO_CONTENT)
}

/// Closes the banned user's live sockets, which also takes them out of voice. Sessions are
/// already gone, so new requests get 401.
fn after_ban(st: &AppState, user: &UserInfo) {
    // At once, not when the socket task gets to it.
    st.voice.kick(&user.user_id);
    st.hub.close_user(&user.user_id, CLOSE_SESSION_ENDED);
    st.hub.update_user(user.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: Role) -> UserInfo {
        UserInfo {
            user_id: "0".repeat(32),
            username: "u".into(),
            display_name: "u".into(),
            accent: String::new(),
            role,
            banned: false,
        }
    }

    #[test]
    fn require_orders_roles() {
        use Role::*;
        for (have, need, ok) in [
            (Member, Member, true),
            (Member, Moderator, false),
            (Member, Owner, false),
            (Moderator, Member, true),
            (Moderator, Moderator, true),
            (Moderator, Owner, false),
            (Owner, Member, true),
            (Owner, Moderator, true),
            (Owner, Owner, true),
        ] {
            assert_eq!(require(&user(have), need).is_ok(), ok, "{have:?} {need:?}");
        }
    }
}
