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
    /// At most 160 characters, one line. Empty when unset.
    pub bio: String,
    /// The picture's version, or null for the letter avatar. Fetch
    /// `/api/users/{user_id}/avatar?v={avatar}`; a new picture gets a new version.
    pub avatar: Option<String>,
    /// The person's chosen UI accent: an `AccentPreset` key or `#rrggbb`. Null means amber, the default.
    /// Not the avatar colour (`accent`).
    pub ui_accent: Option<String>,
}

/// The accent presets of Settings > Appearance. `web/src/lib/ui/accent.ts` holds their colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AccentPreset {
    Amber,
    Rose,
    Orchid,
    Violet,
    Indigo,
    Sky,
    Teal,
    Lime,
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
    /// A reset code that is unknown, used, replaced or expired: all look the same.
    ResetInvalid,
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
    /// 0 to 160 characters; empty clears it.
    #[serde(default)]
    #[ts(optional)]
    pub bio: Option<String>,
    /// An `AccentPreset` key or `#rrggbb`. The client also checks a custom colour's contrast.
    #[serde(default)]
    #[ts(optional)]
    pub ui_accent: Option<String>,
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

/// `POST /api/users/{id}/reset`: the caller's own password, checked again before a link is made.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResetCreate {
    pub password: String,
}

impl std::fmt::Debug for ResetCreate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetCreate")
            .field("password", &"***")
            .finish()
    }
}

/// `GET /api/resets` (owner): for each account its latest reset link, plus every reset used in the
/// last 30 days. Newest first. No code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResetInfo {
    pub user_id: String,
    /// Null: made on the server command line.
    pub created_by: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
    #[ts(type = "number | null")]
    pub used_at: Option<i64>,
    /// `lan` or `internet` when used.
    pub used_from: Option<String>,
    /// Replaced, or ended by a role change, a ban or a password change.
    pub cancelled: bool,
}

/// `POST /api/me/reset-notices`: resets of your password not shown to you yet; asking marks them shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResetNotice {
    /// Display name of who made the link; null for the server command line. Not unique: show
    /// `by_username` with it.
    pub by: Option<String>,
    /// Username (unique) of who made the link; null for the server command line.
    pub by_username: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

/// `POST /api/users/{id}/reset`: a one-use link to set a new password, shown once.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResetCreated {
    pub url: String,
    #[ts(type = "number")]
    pub expires_at: i64,
}

impl std::fmt::Debug for ResetCreated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetCreated")
            .field("url", &"***")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// `POST /api/reset`: the code from the link and the new password (8 to 128 characters).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResetReq {
    pub code: String,
    pub password: String,
}

impl std::fmt::Debug for ResetReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetReq")
            .field("code", &"***")
            .field("password", &"***")
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

/// A room has text and voice. `members` is filled only when `restricted`. An `archived` room is
/// sent to the owner only (for Admin > Rooms); nobody can open, post to, search or join it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoomInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub position: i64,
    pub restricted: bool,
    pub members: Vec<String>,
    pub archived: bool,
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
    /// Unix ms, raised by every edit, delete, reaction and pin change and never equal to an
    /// earlier copy's: of two copies of one message, the larger `updated_at` is the newer.
    #[ts(type = "number")]
    pub updated_at: i64,
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

/// `POST /api/rooms/{id}/uploads`: the stored file, not attached yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UploadDone {
    pub attachment: Attachment,
}

/// `GET /api/admin/disk`, in bytes. `warn` when uploads pass 80% of the quota or the
/// filesystem is over 80% full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiskUse {
    /// The database file plus its WAL.
    #[ts(type = "number")]
    pub db_bytes: i64,
    #[ts(type = "number")]
    pub upload_bytes: i64,
    #[ts(type = "number")]
    pub quota_bytes: i64,
    #[ts(type = "number")]
    pub fs_total: i64,
    #[ts(type = "number")]
    pub fs_free: i64,
    pub warn: bool,
}

/// A slice of a room's history, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub more_before: bool,
    pub more_after: bool,
}

/// `GET /api/search?q=&room_id=&author_id=&from=&to=&before=`. `from` and `to` are unix ms, both
/// inclusive (the client turns local dates into ms). For the next page send `before` =
/// `SearchPage.next_before`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub room_id: Option<i64>,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub from: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub to: Option<i64>,
    /// Page cursor: only messages with a smaller id.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub before: Option<i64>,
}

/// One search result. `snippet` marks each match with `\u0002` before and `\u0003` after; it is
/// plain text, never HTML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    pub message: Message,
    pub snippet: String,
}

/// Newest first. `next_before` is set when there is more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    #[ts(type = "number | null")]
    pub next_before: Option<i64>,
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
    /// The last room that is not archived cannot be archived (409).
    #[serde(default)]
    #[ts(optional)]
    pub archived: Option<bool>,
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

/// `GET /api/push`: the server's VAPID public key (base64url, 65-byte P-256 point), or null when
/// push is off on this server, and the devices of this account that get push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PushInfo {
    pub public_key: Option<String>,
    pub devices: Vec<PushDevice>,
}

/// A browser that gets push for this account. `current` is the one the request came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PushDevice {
    #[ts(type = "number")]
    pub id: i64,
    pub label: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub last_ok_at: Option<i64>,
    pub current: bool,
    pub mentions: bool,
    pub voice: bool,
}

/// `PUT /api/push/subscription`: this browser's push subscription (from
/// `PushSubscription.toJSON()`) and its two switches. Replaces this session's earlier one.
// `Debug` is written by hand: the endpoint is a capability URL and `auth` a secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PushSubscribe {
    pub endpoint: String,
    /// base64url of the 65-byte P-256 public key.
    pub p256dh: String,
    /// base64url of the 16-byte auth secret.
    pub auth: String,
    pub mentions: bool,
    pub voice: bool,
}

impl std::fmt::Debug for PushSubscribe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushSubscribe")
            .field("endpoint", &"***")
            .field("auth", &"***")
            .field("mentions", &self.mentions)
            .field("voice", &self.voice)
            .finish()
    }
}

/// What the service worker gets, encrypted (RFC 8291): never a message body. `kind` is
/// `mention`, `voice` or `test`; `room_id` is 0 for `test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PushPayload {
    pub kind: String,
    #[ts(type = "number")]
    pub room_id: i64,
    pub title: String,
    pub body: String,
}

/// The noise suppression a call ended in (the web client's `DenoiseMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CallMode {
    Deepfilter,
    Browser,
    Off,
}

/// `POST /api/me/calls`: one summary of a call you just left, sent by your own client. Unknown
/// fields are ignored; the server checks every range (see `api_calls.rs`). A figure the client
/// never measured is null. `browser_ns_loading`: the browser filter ran while DeepFilterNet loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallSummary {
    pub duration_s: u32,
    pub mode: CallMode,
    pub fallback_reason: String,
    pub loss_pct: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub concealed_pct: Option<f64>,
    pub rtt_ms: Option<f64>,
    pub browser_ns_loading: bool,
}

/// `GET /api/me/calls`: one of your own recent calls. Times are Unix ms, set by the server
/// (`ended_at` is when the summary arrived).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallRecord {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number")]
    pub ended_at: i64,
    #[serde(flatten)]
    pub summary: CallSummary,
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
            format!(
                "{:?}",
                ResetCreated {
                    url: format!("http://x/reset#{secret}"),
                    expires_at: 0,
                }
            ),
            format!(
                "{:?}",
                ResetCreate {
                    password: secret.into()
                }
            ),
            format!(
                "{:?}",
                ResetReq {
                    code: secret.into(),
                    password: secret.into(),
                }
            ),
            format!(
                "{:?}",
                PushSubscribe {
                    endpoint: format!("https://fcm.googleapis.com/fcm/send/{secret}"),
                    p256dh: "p".into(),
                    auth: secret.into(),
                    mentions: true,
                    voice: false,
                }
            ),
        ];
        for o in out {
            assert!(!o.contains(secret) && o.contains("***"), "{o}");
        }
    }
}
