//! Chat routes: history, pins, send, edit, delete, reactions, pins and read markers. Each one
//! checks `can_see` first: a room you cannot see (or a message in one) is 404, never 403.
//! Changes are pushed to the sockets that can see the room. Message bodies are never logged.
//! Sends have their own rate limit; edits, reactions and pins share one (`Kind::Change`).

use std::collections::HashSet;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;
use vesper_protocol::{
    ApiErrorCode, EditMessage, Message, MessagePage, PinReq, ReactionReq, ReadReq, Role, RoomInfo,
    RoomKind, SearchPage, SearchQuery, SendMessage, ServerMsg, UserInfo,
};
use vesper_store::{Cursor, NewMessage, StoreError, now_ms};

use crate::api_accounts::{body, limit};
use crate::api_admin::{bad, path, require};
use crate::api_rooms::{can_see, visible_room};
use crate::limits::Kind;
use crate::{ApiError, AppState, Authed};

pub const MAX_BODY_CHARS: usize = 4000;
pub const MAX_MENTIONS: usize = 20;
const PAGE: u32 = 50;
const SEARCH_PAGE: u32 = 20;

/// NFC, trimmed, 1 to 4000 chars, no control characters except `\n` and `\t`.
pub fn valid_body(s: &str) -> Option<String> {
    let nfc: String = s.nfc().collect();
    let s = nfc.trim();
    let ok = (1..=MAX_BODY_CHARS).contains(&s.chars().count())
        && !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t');
    ok.then(|| s.to_string())
}

/// `@name` at a word start, `name` being `[a-z0-9_]{3,24}` after an ASCII lowercase (not a
/// Unicode one: the Kelvin sign would become `k`). Deduplicated, first 20.
pub fn mentioned_usernames(body: &str) -> Vec<String> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<char> = body.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() && out.len() < MAX_MENTIONS {
        let start = i == 0 || !(word(chars[i - 1]) || chars[i - 1] == '@');
        if chars[i] != '@' || !start {
            i += 1;
            continue;
        }
        let end = (i + 1..chars.len())
            .find(|&j| !word(chars[j]))
            .unwrap_or(chars.len());
        let name: String = chars[i + 1..end].iter().collect();
        if (3..=24).contains(&name.len()) && !out.contains(&name) {
            out.push(name);
        }
        i = end;
    }
    out
}

/// 1 to 32 bytes with no ASCII letter or digit, whitespace or control character (plan Ruling:
/// no emoji table, odd symbols pass).
pub fn valid_reaction(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && !s
            .chars()
            .any(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c.is_control())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/rooms/{id}/messages", get(history).post(send))
        .route("/rooms/{id}/pins", get(pins))
        .route("/rooms/{id}/read", put(read))
        .route("/messages/{id}", patch(edit).delete(remove))
        .route("/messages/{id}/reaction", put(react))
        .route("/messages/{id}/pin", put(pin))
        .route("/search", get(search))
}

/// The message and its room, if `user` can see the room.
async fn visible_message(
    st: &AppState,
    user: &UserInfo,
    id: i64,
) -> Result<(Message, RoomInfo), ApiError> {
    let msg = st.store.message(id).await?;
    let room = visible_room(st, user, msg.room_id).await?;
    Ok((msg, room))
}

/// User ids of the `@names` in `body` that can see `room` and are not banned.
async fn mentions(st: &AppState, room: &RoomInfo, body: &str) -> Result<Vec<String>, ApiError> {
    let names: HashSet<String> = mentioned_usernames(body).into_iter().collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    Ok(st
        .store
        .users()
        .await?
        .into_iter()
        .filter(|u| names.contains(&u.username) && !u.banned && can_see(u, room))
        .map(|u| u.user_id)
        .collect())
}

fn body_text(s: &str) -> Result<String, ApiError> {
    valid_body(s).ok_or_else(|| {
        bad("message: 1 to 4000 characters, no control characters except newline and tab")
    })
}

#[derive(Deserialize)]
struct HistoryQuery {
    before: Option<i64>,
    after: Option<i64>,
    around: Option<i64>,
    limit: Option<u32>,
}

async fn history(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    q: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<MessagePage>, ApiError> {
    let room = visible_room(&st, &a.user, path(id)?).await?;
    let Query(q) = q.map_err(|_| bad("invalid query"))?;
    let cursor = match (q.before, q.after, q.around) {
        (None, None, None) => Cursor::Latest,
        (Some(id), None, None) => Cursor::Before(id),
        (None, Some(id), None) => Cursor::After(id),
        (None, None, Some(id)) => Cursor::Around(id),
        _ => return Err(bad("use one of before, after and around")),
    };
    let page = st
        .store
        .history(room.id, cursor, q.limit.unwrap_or(PAGE))
        .await?;
    Ok(Json(page))
}

async fn pins(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Vec<Message>>, ApiError> {
    let room = visible_room(&st, &a.user, path(id)?).await?;
    Ok(Json(st.store.pins(room.id).await?))
}

async fn send(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<SendMessage>, JsonRejection>,
) -> Result<(StatusCode, Json<Message>), ApiError> {
    let id = path(id)?;
    let req = body(req)?;
    let room = visible_room(&st, &a.user, id).await?;
    limit(&st, Kind::Message, &a.user.user_id)?;
    let text = body_text(&req.body)?;
    let mentions = mentions(&st, &room, &text).await?;
    let msg = st
        .store
        .post_message(NewMessage {
            room_id: room.id,
            author_id: &a.user.user_id,
            body: &text,
            reply_to: req.reply_to,
            mentions: &mentions,
            attachments: &req.attachments,
            now: now_ms(),
        })
        .await?;
    st.hub.broadcast_room(
        &room,
        ServerMsg::MessageNew {
            message: msg.clone(),
        },
    );
    // Edits do not push: a fixed typo should not ping anyone again.
    let push = st.push.clone();
    if room.kind == RoomKind::Dm {
        // Every DM message is addressed to the other person, so an @mention adds nothing.
        tokio::spawn(async move { push.direct_message(&room, &a.user).await });
    } else if !mentions.is_empty() {
        tokio::spawn(async move { push.mentioned(&room, &a.user, &mentions).await });
    }
    Ok((StatusCode::CREATED, Json(msg)))
}

/// Pushes `MessageUpdated` to the room and gives the message back.
fn updated(st: &AppState, room: &RoomInfo, msg: Message) -> Message {
    st.hub.broadcast_room(
        room,
        ServerMsg::MessageUpdated {
            message: msg.clone(),
        },
    );
    msg
}

async fn edit(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<EditMessage>, JsonRejection>,
) -> Result<Json<Message>, ApiError> {
    let id = path(id)?;
    let req = body(req)?;
    let (msg, room) = visible_message(&st, &a.user, id).await?;
    if msg.author_id != a.user.user_id {
        return Err(ApiError::new(ApiErrorCode::Forbidden, "not allowed"));
    }
    limit(&st, Kind::Change, &a.user.user_id)?;
    let text = body_text(&req.body)?;
    let mentions = mentions(&st, &room, &text).await?;
    let msg = st
        .store
        .edit_message(id, &a.user.user_id, &text, &mentions, now_ms())
        .await?;
    Ok(Json(updated(&st, &room, msg)))
}

/// The author, or a moderator or the owner, deletes.
async fn remove(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    let (msg, room) = visible_message(&st, &a.user, id).await?;
    if msg.author_id != a.user.user_id {
        require(&a.user, Role::Moderator)?;
    }
    let (msg, uploads) = st
        .store
        .delete_message(id, now_ms(), &a.user.user_id)
        .await?;
    crate::uploads::remove_files(&st.cfg.data_dir, &uploads).await;
    updated(&st, &room, msg);
    Ok(StatusCode::NO_CONTENT)
}

async fn react(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<ReactionReq>, JsonRejection>,
) -> Result<Json<Message>, ApiError> {
    let id = path(id)?;
    let req = body(req)?;
    let (_, room) = visible_message(&st, &a.user, id).await?;
    if !valid_reaction(&req.emoji) {
        return Err(bad("reaction: 1 to 32 bytes, no letters, digits or spaces"));
    }
    limit(&st, Kind::Change, &a.user.user_id)?;
    let msg = st
        .store
        .set_reaction(id, &a.user.user_id, &req.emoji, req.on, now_ms())
        .await?;
    Ok(Json(updated(&st, &room, msg)))
}

async fn pin(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<PinReq>, JsonRejection>,
) -> Result<Json<Message>, ApiError> {
    let id = path(id)?;
    let req = body(req)?;
    let (_, room) = visible_message(&st, &a.user, id).await?;
    limit(&st, Kind::Change, &a.user.user_id)?;
    let msg = st
        .store
        .set_pinned(id, &a.user.user_id, req.on, now_ms())
        .await?;
    Ok(Json(updated(&st, &room, msg)))
}

/// Moves the marker forward. It must name a message of that room (0 means none read), so a
/// client cannot park it past every future message.
async fn read(
    State(st): State<AppState>,
    a: Authed,
    id: Result<Path<i64>, PathRejection>,
    req: Result<Json<ReadReq>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path(id)?;
    let req = body(req)?;
    let room = visible_room(&st, &a.user, id).await?;
    let ok = req.last_read_id == 0
        || match st.store.message(req.last_read_id).await {
            Ok(m) => m.room_id == room.id,
            Err(StoreError::NotFound) => false,
            // A database error is a logged 500, not the client's fault.
            Err(e) => return Err(e.into()),
        };
    if !ok {
        return Err(bad("last_read_id: not a message of this room"));
    }
    let me = &a.user.user_id;
    st.store.mark_read(me, room.id, req.last_read_id).await?;
    // The marker never moves back, so tell the other tabs where it really is.
    let now = st.store.unread(me, &[room.id]).await?;
    if let Some(u) = now.first() {
        st.hub.send_user(
            me,
            ServerMsg::Read {
                room_id: room.id,
                last_read_id: u.last_read_id,
            },
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Search over the rooms the user can see, worked out here and applied inside the store's SQL.
/// A `room_id` the user cannot see is the same 404 as an unknown room.
async fn search(
    State(st): State<AppState>,
    a: Authed,
    q: Result<Query<SearchQuery>, QueryRejection>,
) -> Result<Json<SearchPage>, ApiError> {
    let Query(q) = q.map_err(|_| bad("invalid query"))?;
    limit(&st, Kind::Search, &a.user.user_id)?;
    if let Some(id) = q.room_id {
        visible_room(&st, &a.user, id).await?;
    }
    let visible: Vec<i64> = st
        .store
        .rooms()
        .await?
        .iter()
        .filter(|r| can_see(&a.user, r))
        .map(|r| r.id)
        .collect();
    Ok(Json(st.store.search(&q, &visible, SEARCH_PAGE).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_rules() {
        assert_eq!(
            valid_body("  hi\n\tthere  ").as_deref(),
            Some("hi\n\tthere")
        );
        assert_eq!(
            valid_body(&"é".repeat(4000)).map(|s| s.chars().count()),
            Some(4000)
        );
        assert!(valid_body(&"a".repeat(4001)).is_none());
        for bad in ["", " \n\t ", "a\u{0}b", "a\rb", "a\u{7f}", "a\u{1b}[31m"] {
            assert!(valid_body(bad).is_none(), "{bad:?}");
        }
        // NFC: e + combining acute becomes one char.
        assert_eq!(valid_body("e\u{301}").as_deref(), Some("\u{e9}"));
    }

    #[test]
    fn mention_rules() {
        assert_eq!(
            mentioned_usernames("hi @Bob, @bob and @carol_1!"),
            ["bob", "carol_1"]
        );
        // Not at a word start, too short, too long, a Unicode lookalike.
        let long = format!("@{}", "a".repeat(25));
        for none in [
            "a@bob",
            "@@bob",
            "@ab",
            long.as_str(),
            "@\u{212a}ate",
            "_@bob",
        ] {
            assert!(mentioned_usernames(none).is_empty(), "{none}");
        }
        assert_eq!(mentioned_usernames("(@bob) @bob.x"), ["bob"]);
        let many: String = (0..30).map(|i| format!("@user{i:02} ")).collect();
        assert_eq!(mentioned_usernames(&many).len(), MAX_MENTIONS);
    }

    #[test]
    fn reaction_rules() {
        for ok in ["👍", "❤️", "👨‍👩‍👧", "!"] {
            assert!(valid_reaction(ok), "{ok}");
        }
        for bad in [
            "",
            "ab",
            " ",
            "1",
            "👍 ",
            "\u{3000}",
            "\u{0}",
            &"👍".repeat(9),
        ] {
            assert!(!valid_reaction(bad), "{bad:?}");
        }
    }
}
