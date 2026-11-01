//! REST bodies and the structs shared by the REST API and the store.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Role {
    Owner,
    Moderator,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UserInfo {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub accent: String,
    pub role: Role,
    pub banned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Device {
    #[ts(type = "number")]
    pub id: i64,
    pub label: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub last_seen_at: i64,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InviteInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub role: Role,
    pub uses: u32,
    pub max_uses: u32,
    #[ts(type = "number")]
    pub expires_at: i64,
    pub created_by: Option<String>,
    pub revoked: bool,
}

/// Error body of every failed `/api` call. The HTTP status follows `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApiErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    TooLarge,
    QuotaFull,
    InviteInvalid,
    Internal,
}

// `Debug` is written by hand so the password and invite code never reach a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SignupReq {
    pub invite: String,
    pub username: String,
    pub password: String,
    pub display_name: String,
}

// `Debug` is written by hand so the password and invite code never reach a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for SignupReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignupReq")
            .field("invite", &"***")
            .field("username", &self.username)
            .field("password", &"***")
            .field("display_name", &self.display_name)
            .finish()
    }
}

impl std::fmt::Debug for LoginReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginReq")
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

/// A missing field is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProfilePatch {
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub accent: Option<String>,
}

// `Debug` is written by hand so the password and invite code never reach a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasswordChange {
    pub old: String,
    pub new: String,
}

impl std::fmt::Debug for PasswordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordChange")
            .field("old", &"***")
            .field("new", &"***")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Me {
    pub user: UserInfo,
    #[ts(type = "number")]
    pub session_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InvitePreview {
    pub role: Role,
}

/// `POST /api/invites`. `max_uses` 1 to 50, `expires_hours` 1 to 720, `role` not `owner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InviteCreate {
    pub role: Role,
    pub max_uses: u32,
    pub expires_hours: u32,
}

// `Debug` is written by hand so the invite code in `url` never reaches a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InviteCreated {
    pub info: InviteInfo,
    pub url: String,
}

impl std::fmt::Debug for InviteCreated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteCreated")
            .field("info", &self.info)
            .field("url", &"***")
            .finish()
    }
}

/// `PATCH /api/users/{id}/role`. `role` not `owner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RolePatch {
    pub role: Role,
}

/// `POST /api/users/{id}/force-mute`. `muted: false` lifts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForceMute {
    pub muted: bool,
}

/// A room has text and voice. `members` is filled only when `restricted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub position: i64,
    pub restricted: bool,
    pub members: Vec<String>,
}

/// Unread state of one room for the signed-in user, sent in `Hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Unread {
    #[ts(type = "number")]
    pub room_id: i64,
    #[ts(type = "number")]
    pub last_read_id: i64,
    pub unread: u32,
    pub mentions: u32,
}

/// A chat message. A deleted one keeps its id and times with an empty body and no reactions,
/// attachments, mentions or pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Message {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub room_id: i64,
    pub author_id: String,
    pub body: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub edited_at: Option<i64>,
    pub deleted: bool,
    pub reply_to: Option<ReplyPreview>,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<Attachment>,
    /// User ids mentioned in the body.
    pub mentions: Vec<String>,
    pub pinned: bool,
}

/// The message a reply points at. `excerpt` is its first 100 characters, empty when it was deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReplyPreview {
    #[ts(type = "number")]
    pub id: i64,
    pub author_id: String,
    pub excerpt: String,
}

/// One emoji on a message and who added it, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reaction {
    pub emoji: String,
    pub user_ids: Vec<String>,
}

/// An uploaded file attached to a message. `image` means it is shown inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    #[ts(type = "number")]
    pub size: i64,
    pub image: bool,
}

/// A slice of a room's history, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub more_before: bool,
    pub more_after: bool,
}

/// `POST /api/rooms/{id}/messages`. `attachments` are upload ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SendMessage {
    pub body: String,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub reply_to: Option<i64>,
    #[serde(default)]
    pub attachments: Vec<String>,
}

/// `PATCH /api/messages/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditMessage {
    pub body: String,
}

/// `PUT /api/messages/{id}/reaction`: add (`on`) or remove your own reaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReactionReq {
    pub emoji: String,
    pub on: bool,
}

/// `PUT /api/messages/{id}/pin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PinReq {
    pub on: bool,
}

/// `PUT /api/rooms/{id}/read`: the newest message id you have seen in that room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadReq {
    #[ts(type = "number")]
    pub last_read_id: i64,
}

/// `POST /api/rooms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomCreate {
    pub name: String,
}

/// `PATCH /api/rooms/{id}`. A missing field is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomPatch {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub restricted: Option<bool>,
}

/// `PUT /api/rooms/order`: every room id once, in the new order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomOrder {
    #[ts(type = "number[]")]
    pub ids: Vec<i64>,
}

/// `PUT /api/rooms/{id}/members`: the whole member list, replacing the old one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomMembers {
    pub user_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_passwords_and_invite() {
        let secret = "s3cret-value";
        let out = [
            format!(
                "{:?}",
                SignupReq {
                    invite: secret.into(),
                    username: "alice".into(),
                    password: secret.into(),
                    display_name: "A".into(),
                }
            ),
            format!(
                "{:?}",
                LoginReq {
                    username: "alice".into(),
                    password: secret.into(),
                }
            ),
            format!(
                "{:?}",
                PasswordChange {
                    old: secret.into(),
                    new: secret.into(),
                }
            ),
            format!(
                "{:?}",
                InviteCreated {
                    info: InviteInfo {
                        id: 1,
                        role: Role::Member,
                        uses: 0,
                        max_uses: 1,
                        expires_at: 0,
                        created_by: None,
                        revoked: false,
                    },
                    url: format!("http://x/invite#{secret}"),
                }
            ),
        ];
        for o in out {
            assert!(!o.contains(secret) && o.contains("***"), "{o}");
        }
    }
}
