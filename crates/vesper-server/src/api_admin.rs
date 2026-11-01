//! Admin routes: the user list, invites, roles and bans.
//! Owner > moderator > member. Moderators mint and revoke member invites; only the owner mints
//! moderator invites, changes roles and bans. The owner role only comes from the CLI, so it can
//! never be granted, taken away or banned here.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use vesper_protocol::{
    ApiErrorCode, InviteCreate, InviteCreated, InviteInfo, Role, RolePatch, UserInfo,
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

fn bad(message: &str) -> ApiError {
    ApiError::new(ApiErrorCode::BadRequest, message)
}

fn path<T>(p: Result<Path<T>, PathRejection>) -> Result<T, ApiError> {
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
    st.store.revoke_invite(path(id)?, now_ms()).await?;
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
    Ok(Json(st.store.set_role(&id, req.role).await?))
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
    after_ban(&st, &id);
    Ok(Json(user))
}

async fn unban(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<String>, PathRejection>,
) -> Result<Json<UserInfo>, ApiError> {
    let id = path(id)?;
    owner_target(&st, &a, &id).await?;
    Ok(Json(st.store.set_banned(&id, false, now_ms()).await?))
}

/// Closes the banned user's live sockets. Sessions are already gone, so new requests get 401.
fn after_ban(_state: &AppState, _user_id: &str) {
    // Task 8: hub.disconnect_user
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
