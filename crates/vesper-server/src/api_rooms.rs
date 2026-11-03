//! Room routes. Everyone signed in lists the rooms they can see; only the owner creates, renames,
//! reorders, restricts and sets member lists (plan Ruling: rooms are not a moderator power).

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use vesper_protocol::{
    DmCreate, Role, RoomCreate, RoomInfo, RoomKind, RoomMembers, RoomOrder, RoomPatch, UserInfo,
};
use vesper_store::now_ms;

use crate::api_accounts::body;
use crate::api_admin::{bad, path, require};
use crate::auth::valid_display_name;
use crate::{ApiError, AppState, Authed};

/// Owner and moderators see every room; others see open rooms and restricted ones that list them.
/// Nobody sees an archived room: it can not be read, posted to, searched, typed in or joined.
/// A DM is the exception to the role rule: only its two members see it, whatever their role.
pub fn can_see(user: &UserInfo, room: &RoomInfo) -> bool {
    if room.archived {
        return false;
    }
    let member = room.members.contains(&user.user_id);
    match room.kind {
        RoomKind::Dm => member,
        RoomKind::Room => {
            matches!(user.role, Role::Owner | Role::Moderator) || !room.restricted || member
        }
    }
}

/// In the user's room list: what `can_see` allows, plus archived rooms for the owner, whose
/// Admin > Rooms lists them to unarchive.
pub fn listed(user: &UserInfo, room: &RoomInfo) -> bool {
    can_see(user, room) || (room.archived && user.role == Role::Owner)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rooms", get(list).post(create))
        .route("/rooms/order", put(reorder))
        .route("/rooms/{id}", patch(update))
        .route("/rooms/{id}/members", put(members))
        .route("/dms", post(open_dm))
}

/// Same rule as display names: NFC, 1 to 32 chars, no control or hidden format characters.
fn room_name(s: &str) -> Result<String, ApiError> {
    valid_display_name(s).ok_or_else(|| bad("room name: 1 to 32 characters, no hidden characters"))
}

/// Tells connected clients the room list changed, each with only the rooms they can see, and
/// takes out of voice anyone who lost sight of their room (member list, role change or archive).
pub(crate) async fn rooms_changed(st: &AppState) {
    match tokio::try_join!(st.store.rooms(), st.store.users()) {
        Ok((rooms, users)) => {
            st.voice.refresh(&rooms, &users);
            st.hub.rooms_changed(&rooms);
        }
        Err(e) => tracing::error!("store: {e}"),
    }
}

async fn list(State(st): State<AppState>, a: Authed) -> Result<Json<Vec<RoomInfo>>, ApiError> {
    let mut rooms = st.store.rooms().await?;
    rooms.retain(|r| listed(&a.user, r));
    Ok(Json(rooms))
}

async fn create(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<RoomCreate>, JsonRejection>,
) -> Result<(StatusCode, Json<RoomInfo>), ApiError> {
    require(&a.user, Role::Owner)?;
    let name = room_name(&body(req)?.name)?;
    let room = st.store.create_room(&name, now_ms()).await?;
    rooms_changed(&st).await;
    Ok((StatusCode::CREATED, Json(room)))
}

async fn update(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<RoomPatch>, JsonRejection>,
) -> Result<Json<RoomInfo>, ApiError> {
    require(&a.user, Role::Owner)?;
    let id = path(id)?;
    let req = body(req)?;
    let name = req.name.as_deref().map(room_name).transpose()?;
    let room = st
        .store
        .patch_room(id, name.as_deref(), req.restricted, req.archived, now_ms())
        .await?;
    rooms_changed(&st).await;
    Ok(Json(room))
}

async fn reorder(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<RoomOrder>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    require(&a.user, Role::Owner)?;
    st.store.reorder_rooms(&body(req)?.ids).await?;
    rooms_changed(&st).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn members(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<RoomMembers>, JsonRejection>,
) -> Result<Json<RoomInfo>, ApiError> {
    require(&a.user, Role::Owner)?;
    let id = path(id)?;
    let ids = body(req)?.user_ids;
    // A banned account can not be listed (unknown ids are 404 from the store).
    if st
        .store
        .users()
        .await?
        .iter()
        .any(|u| u.banned && ids.contains(&u.user_id))
    {
        return Err(bad("a banned account can not be a member"));
    }
    let room = st.store.set_room_members(id, &ids).await?;
    rooms_changed(&st).await;
    Ok(Json(room))
}

/// Opens (or makes) the DM with another person. A banned person can not be messaged; a banned
/// account's existing DMs stay readable to the other person.
async fn open_dm(
    State(st): State<AppState>,
    a: Authed,
    req: Result<Json<DmCreate>, JsonRejection>,
) -> Result<Json<RoomInfo>, ApiError> {
    crate::api_accounts::limit(&st, crate::limits::Kind::Change, &a.user.user_id)?;
    let other = body(req)?.user_id;
    if other == a.user.user_id {
        return Err(bad("you can not message yourself"));
    }
    if st.store.user(&other).await?.banned {
        return Err(bad("that account is banned"));
    }
    let room = st.store.dm_with(&a.user.user_id, &other, now_ms()).await?;
    rooms_changed(&st).await;
    Ok(Json(room))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_see_by_role_and_list() {
        let user = |role, id: &str| UserInfo {
            user_id: id.into(),
            username: "u".into(),
            display_name: "u".into(),
            accent: String::new(),
            role,
            banned: false,
            bio: String::new(),
            avatar: None,
            ui_accent: None,
        };
        let room = |restricted, members: &[&str]| RoomInfo {
            id: 1,
            name: "r".into(),
            position: 0,
            restricted,
            members: members.iter().map(|m| m.to_string()).collect(),
            ..RoomInfo::default()
        };
        let (open, closed, named) = (room(false, &[]), room(true, &["b"]), room(true, &["a"]));
        for role in [Role::Owner, Role::Moderator, Role::Member] {
            let u = user(role, "a");
            let staff = role != Role::Member;
            assert!(can_see(&u, &open), "{role:?}");
            assert_eq!(can_see(&u, &closed), staff, "{role:?}");
            assert!(can_see(&u, &named), "{role:?}");
            for r in [&open, &closed, &named] {
                let gone = RoomInfo {
                    archived: true,
                    ..r.clone()
                };
                assert!(!can_see(&u, &gone), "{role:?}");
                assert_eq!(listed(&u, &gone), role == Role::Owner, "{role:?}");
                assert_eq!(listed(&u, r), can_see(&u, r), "{role:?}");
            }
            // A DM is seen by its two members only, whatever their role.
            let dm = |members: &[&str]| RoomInfo {
                kind: RoomKind::Dm,
                restricted: true,
                members: members.iter().map(|m| m.to_string()).collect(),
                ..room(true, &[])
            };
            assert!(can_see(&u, &dm(&["a", "b"])), "{role:?}");
            assert!(!can_see(&u, &dm(&["b", "c"])), "{role:?}");
            let gone = RoomInfo {
                archived: true,
                ..dm(&["a", "b"])
            };
            assert!(!can_see(&u, &gone), "{role:?}");
            assert!(!listed(&u, &dm(&["b", "c"])), "{role:?}");
        }
    }
}
