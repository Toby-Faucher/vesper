use std::collections::HashMap;

use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::{Attachment, Message, MessagePage, Reaction, ReplyPreview, Unread};

use crate::{Result, Store, StoreError, constraint};

/// Validated by the server (body rules, mentions resolved to user ids) before it gets here.
pub struct NewMessage<'a> {
    pub room_id: i64,
    pub author_id: &'a str,
    pub body: &'a str,
    pub reply_to: Option<i64>,
    /// User ids.
    pub mentions: &'a [String],
    /// Upload ids: each must be the author's, in the same room and not attached yet.
    pub attachments: &'a [String],
    pub now: i64,
}

/// Where a history page starts. Ids only grow, so a cursor never skips or repeats a message when
/// others are posted meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Latest,
    Before(i64),
    After(i64),
    Around(i64),
}

pub const MAX_REACTIONS: i64 = 20;
pub const MAX_PINS: i64 = 50;

macro_rules! msg_select {
    () => {
        "SELECT m.id, m.room_id, m.author_id, m.body, m.created_at, m.edited_at,
                m.deleted_at IS NOT NULL AS deleted, m.pinned_at IS NOT NULL AS pinned,
                p.id AS reply_id, p.author_id AS reply_author, substr(p.body, 1, 100) AS reply_excerpt
         FROM messages m LEFT JOIN messages p ON p.id = m.reply_to "
    };
}

const BEFORE: &str = concat!(
    msg_select!(),
    "WHERE m.room_id = ?1 AND m.id < ?2 ORDER BY m.id DESC LIMIT ?3"
);
/// Messages whose ids are in the JSON array `?1`, any order.
pub(crate) const BY_IDS: &str = concat!(
    msg_select!(),
    "WHERE m.id IN (SELECT value FROM json_each(?1))"
);
const AFTER: &str = concat!(
    msg_select!(),
    "WHERE m.room_id = ?1 AND m.id > ?2 ORDER BY m.id ASC LIMIT ?3"
);

#[derive(FromRow)]
pub(crate) struct MsgRow {
    id: i64,
    room_id: i64,
    author_id: String,
    body: String,
    created_at: i64,
    edited_at: Option<i64>,
    deleted: bool,
    pinned: bool,
    reply_id: Option<i64>,
    reply_author: Option<String>,
    reply_excerpt: Option<String>,
}

pub(crate) fn json_ids(ids: impl IntoIterator<Item = i64>) -> String {
    let parts: Vec<String> = ids.into_iter().map(|i| i.to_string()).collect();
    format!("[{}]", parts.join(","))
}

fn json_strs(s: &[String]) -> String {
    serde_json::to_string(s).expect("strings serialise")
}

/// Task 14 must store the sniffed type as `mime` (or `application/octet-stream`) for this to hold.
fn is_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// Fills reactions, mentions and attachments with one query each.
pub(crate) async fn load(c: &mut SqliteConnection, rows: Vec<MsgRow>) -> Result<Vec<Message>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = json_ids(rows.iter().map(|r| r.id));
    let mut reactions: HashMap<i64, Vec<Reaction>> = HashMap::new();
    let pairs: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT message_id, emoji, account_id FROM reactions
         WHERE message_id IN (SELECT value FROM json_each(?1))
         ORDER BY message_id, created_at, account_id",
    )
    .bind(&ids)
    .fetch_all(&mut *c)
    .await?;
    for (msg, emoji, user) in pairs {
        let list = reactions.entry(msg).or_default();
        match list.iter_mut().find(|r| r.emoji == emoji) {
            Some(r) => r.user_ids.push(user),
            None => list.push(Reaction {
                emoji,
                user_ids: vec![user],
            }),
        }
    }
    let mut mentions: HashMap<i64, Vec<String>> = HashMap::new();
    let pairs: Vec<(i64, String)> = sqlx::query_as(
        "SELECT message_id, account_id FROM mentions
         WHERE message_id IN (SELECT value FROM json_each(?1)) ORDER BY account_id",
    )
    .bind(&ids)
    .fetch_all(&mut *c)
    .await?;
    for (msg, user) in pairs {
        mentions.entry(msg).or_default().push(user);
    }
    let mut attachments: HashMap<i64, Vec<Attachment>> = HashMap::new();
    let files: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT message_id, id, name, mime, size FROM uploads
         WHERE message_id IN (SELECT value FROM json_each(?1)) ORDER BY created_at, id",
    )
    .bind(&ids)
    .fetch_all(&mut *c)
    .await?;
    for (msg, id, name, mime, size) in files {
        attachments.entry(msg).or_default().push(Attachment {
            image: is_image(&mime),
            id,
            name,
            mime,
            size,
        });
    }
    Ok(rows
        .into_iter()
        .map(|r| Message {
            reactions: reactions.remove(&r.id).unwrap_or_default(),
            mentions: mentions.remove(&r.id).unwrap_or_default(),
            attachments: attachments.remove(&r.id).unwrap_or_default(),
            reply_to: match (r.reply_id, r.reply_author) {
                (Some(id), Some(author_id)) => Some(ReplyPreview {
                    id,
                    author_id,
                    excerpt: r.reply_excerpt.unwrap_or_default(),
                }),
                _ => None,
            },
            id: r.id,
            room_id: r.room_id,
            author_id: r.author_id,
            body: r.body,
            created_at: r.created_at,
            edited_at: r.edited_at,
            deleted: r.deleted,
            pinned: r.pinned,
        })
        .collect())
}

async fn by_id(c: &mut SqliteConnection, id: i64) -> Result<Message> {
    let row = sqlx::query_as::<_, MsgRow>(concat!(msg_select!(), "WHERE m.id = ?1"))
        .bind(id)
        .fetch_optional(&mut *c)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(load(c, vec![row]).await?.remove(0))
}

/// `n` rows of `room` past `from` in the direction of `sql` (BEFORE is newest first).
async fn rows(
    c: &mut SqliteConnection,
    sql: &'static str,
    room: i64,
    from: i64,
    n: u32,
) -> Result<Vec<MsgRow>> {
    Ok(sqlx::query_as::<_, MsgRow>(sql)
        .bind(room)
        .bind(from)
        .bind(n)
        .fetch_all(&mut *c)
        .await?)
}

async fn exists(c: &mut SqliteConnection, sql: &'static str, room: i64, id: i64) -> Result<bool> {
    Ok(sqlx::query_scalar(sql)
        .bind(room)
        .bind(id)
        .fetch_one(&mut *c)
        .await?)
}

const ANY_BEFORE: &str = "SELECT EXISTS (SELECT 1 FROM messages WHERE room_id = ?1 AND id < ?2)";
const ANY_AFTER: &str = "SELECT EXISTS (SELECT 1 FROM messages WHERE room_id = ?1 AND id > ?2)";

/// Fetches up to `n` rows and says whether more lie beyond them.
async fn side(
    c: &mut SqliteConnection,
    sql: &'static str,
    room: i64,
    from: i64,
    n: u32,
) -> Result<(Vec<MsgRow>, bool)> {
    let mut r = rows(c, sql, room, from, n + 1).await?;
    let more = r.len() > n as usize;
    r.truncate(n as usize);
    Ok((r, more))
}

async fn set_mentions(c: &mut SqliteConnection, id: i64, users: &[String]) -> Result<()> {
    sqlx::query("DELETE FROM mentions WHERE message_id = ?1")
        .bind(id)
        .execute(&mut *c)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO mentions (account_id, message_id)
         SELECT value, ?1 FROM json_each(?2)",
    )
    .bind(id)
    .bind(json_strs(users))
    .execute(&mut *c)
    .await
    .map_err(|e| constraint(e, "duplicate mention"))?;
    Ok(())
}

impl Store {
    /// `reply_to` outside the room (or unknown) and an unusable attachment are `Conflict`; an
    /// unknown room, author or mentioned user is `NotFound`. Nothing is written on error.
    pub async fn post_message(&self, m: NewMessage<'_>) -> Result<Message> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(reply) = m.reply_to {
            let same: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1 AND room_id = ?2)",
            )
            .bind(reply)
            .bind(m.room_id)
            .fetch_one(&mut *tx)
            .await?;
            if !same {
                return Err(StoreError::Conflict("reply must be in the same room"));
            }
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO messages (room_id, author_id, body, reply_to, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
        )
        .bind(m.room_id)
        .bind(m.author_id)
        .bind(m.body)
        .bind(m.reply_to)
        .bind(m.now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| constraint(e, "message exists"))?;
        set_mentions(&mut tx, id, m.mentions).await?;
        for upload in m.attachments {
            let n = sqlx::query(
                "UPDATE uploads SET message_id = ?1
                 WHERE id = ?2 AND uploader_id = ?3 AND room_id = ?4 AND message_id IS NULL",
            )
            .bind(id)
            .bind(upload)
            .bind(m.author_id)
            .bind(m.room_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if n == 0 {
                return Err(StoreError::Conflict("attachment not available"));
            }
        }
        let msg = by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(msg)
    }

    /// Only the author edits, and not after a delete: both are `NotFound`. Mentions are replaced.
    pub async fn edit_message(
        &self,
        id: i64,
        author_id: &str,
        body: &str,
        mentions: &[String],
        now: i64,
    ) -> Result<Message> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let n = sqlx::query(
            "UPDATE messages SET body = ?3, edited_at = ?4
             WHERE id = ?1 AND author_id = ?2 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(author_id)
        .bind(body)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        set_mentions(&mut tx, id, mentions).await?;
        let msg = by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(msg)
    }

    /// Soft delete: the body is emptied and reactions, mentions, pin and uploads go. Returns the
    /// removed upload ids so the caller deletes their files. The caller checks who may delete.
    /// Unknown or already deleted is `NotFound`.
    pub async fn delete_message(
        &self,
        id: i64,
        now: i64,
        by: &str,
    ) -> Result<(Message, Vec<String>)> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let n = sqlx::query(
            "UPDATE messages SET body = '', deleted_at = ?2, deleted_by = ?3,
                    pinned_at = NULL, pinned_by = NULL
             WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .bind(by)
        .execute(&mut *tx)
        .await
        .map_err(|e| constraint(e, "message deleted"))?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        for sql in [
            "DELETE FROM reactions WHERE message_id = ?1",
            "DELETE FROM mentions WHERE message_id = ?1",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        }
        let uploads: Vec<String> =
            sqlx::query_scalar("DELETE FROM uploads WHERE message_id = ?1 RETURNING id")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        let msg = by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok((msg, uploads))
    }

    /// Adds or removes one user's emoji (both idempotent). A 21st distinct emoji is `Conflict`;
    /// a deleted message is `NotFound`. The caller validates the emoji.
    pub async fn set_reaction(
        &self,
        id: i64,
        user_id: &str,
        emoji: &str,
        on: bool,
        now: i64,
    ) -> Result<Message> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1 AND deleted_at IS NULL)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !live {
            return Err(StoreError::NotFound);
        }
        if on {
            let (distinct, present): (i64, bool) = sqlx::query_as(
                "SELECT count(DISTINCT emoji), coalesce(max(emoji = ?2), 0)
                 FROM reactions WHERE message_id = ?1",
            )
            .bind(id)
            .bind(emoji)
            .fetch_one(&mut *tx)
            .await?;
            if !present && distinct >= MAX_REACTIONS {
                return Err(StoreError::Conflict("too many different reactions"));
            }
            sqlx::query(
                "INSERT OR IGNORE INTO reactions (message_id, account_id, emoji, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(id)
            .bind(user_id)
            .bind(emoji)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| constraint(e, "duplicate reaction"))?;
        } else {
            sqlx::query(
                "DELETE FROM reactions WHERE message_id = ?1 AND account_id = ?2 AND emoji = ?3",
            )
            .bind(id)
            .bind(user_id)
            .bind(emoji)
            .execute(&mut *tx)
            .await?;
        }
        let msg = by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(msg)
    }

    /// Pins or unpins (both idempotent). A 51st pin in the room is `Conflict`; a deleted message
    /// is `NotFound`.
    pub async fn set_pinned(&self, id: i64, by: &str, on: bool, now: i64) -> Result<Message> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let (room, pinned): (i64, bool) = sqlx::query_as(
            "SELECT room_id, pinned_at IS NOT NULL FROM messages WHERE id = ?1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        if on && !pinned {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM messages WHERE room_id = ?1 AND pinned_at IS NOT NULL",
            )
            .bind(room)
            .fetch_one(&mut *tx)
            .await?;
            if count >= MAX_PINS {
                return Err(StoreError::Conflict("this room has 50 pins"));
            }
            sqlx::query("UPDATE messages SET pinned_at = ?2, pinned_by = ?3 WHERE id = ?1")
                .bind(id)
                .bind(now)
                .bind(by)
                .execute(&mut *tx)
                .await
                .map_err(|e| constraint(e, "pin"))?;
        } else if !on && pinned {
            sqlx::query("UPDATE messages SET pinned_at = NULL, pinned_by = NULL WHERE id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let msg = by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(msg)
    }

    pub async fn message(&self, id: i64) -> Result<Message> {
        let mut c = self.pool.acquire().await?;
        by_id(&mut c, id).await
    }

    /// One page, oldest first, `limit` clamped to 1..=100. `Around(id)` gives `limit / 2` on each
    /// side plus the target, and `NotFound` when the target is not in the room. Deleted messages
    /// stay in as tombstones.
    pub async fn history(&self, room_id: i64, cursor: Cursor, limit: u32) -> Result<MessagePage> {
        let limit = limit.clamp(1, 100);
        let mut c = self.pool.acquire().await?;
        let (before, more_before, after, more_after) = match cursor {
            Cursor::Latest | Cursor::Before(_) => {
                let from = match cursor {
                    Cursor::Before(id) => id,
                    _ => i64::MAX,
                };
                let (b, more) = side(&mut c, BEFORE, room_id, from, limit).await?;
                let later = exists(&mut c, ANY_AFTER, room_id, from.saturating_sub(1)).await?;
                (b, more, Vec::new(), later)
            }
            Cursor::After(from) => {
                let (a, more) = side(&mut c, AFTER, room_id, from, limit).await?;
                let earlier = exists(&mut c, ANY_BEFORE, room_id, from.saturating_add(1)).await?;
                (Vec::new(), earlier, a, more)
            }
            Cursor::Around(id) => {
                let target = sqlx::query_as::<_, MsgRow>(concat!(
                    msg_select!(),
                    "WHERE m.id = ?1 AND m.room_id = ?2"
                ))
                .bind(id)
                .bind(room_id)
                .fetch_optional(&mut *c)
                .await?
                .ok_or(StoreError::NotFound)?;
                let half = limit / 2;
                let (b, more_b) = side(&mut c, BEFORE, room_id, id, half).await?;
                let (mut a, more_a) = side(&mut c, AFTER, room_id, id, half).await?;
                a.insert(0, target);
                (b, more_b, a, more_a)
            }
        };
        let page: Vec<MsgRow> = before.into_iter().rev().chain(after).collect();
        Ok(MessagePage {
            messages: load(&mut c, page).await?,
            more_before,
            more_after,
        })
    }

    /// Pinned messages of a room, newest pin first.
    pub async fn pins(&self, room_id: i64) -> Result<Vec<Message>> {
        let mut c = self.pool.acquire().await?;
        let rows = sqlx::query_as::<_, MsgRow>(concat!(
            msg_select!(),
            "WHERE m.room_id = ?1 AND m.pinned_at IS NOT NULL ORDER BY m.pinned_at DESC, m.id DESC"
        ))
        .bind(room_id)
        .fetch_all(&mut *c)
        .await?;
        load(&mut c, rows).await
    }

    /// Moves the read marker forward; an older id is ignored. Unknown user or room is `NotFound`.
    pub async fn mark_read(&self, user_id: &str, room_id: i64, last_read_id: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO reads (account_id, room_id, last_read_id) VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id, room_id)
             DO UPDATE SET last_read_id = max(last_read_id, excluded.last_read_id)",
        )
        .bind(user_id)
        .bind(room_id)
        .bind(last_read_id)
        .execute(&self.pool)
        .await
        .map_err(|e| constraint(e, "read marker"))?;
        Ok(())
    }

    /// Per room, in the order given: live messages from others past the read marker, and how many
    /// of them mention the user. Both counts are range scans on an index.
    pub async fn unread(&self, user_id: &str, room_ids: &[i64]) -> Result<Vec<Unread>> {
        let rows: Vec<(i64, i64, u32, u32)> = sqlx::query_as(
            "WITH r AS (
                 SELECT j.value AS room_id, coalesce(rd.last_read_id, 0) AS last_read_id, j.key AS k
                 FROM json_each(?2) j
                 LEFT JOIN reads rd ON rd.account_id = ?1 AND rd.room_id = j.value
             )
             SELECT r.room_id, r.last_read_id,
                 (SELECT count(*) FROM messages m
                  WHERE m.room_id = r.room_id AND m.id > r.last_read_id
                    AND m.deleted_at IS NULL AND m.author_id <> ?1),
                 (SELECT count(*) FROM mentions n JOIN messages m ON m.id = n.message_id
                  WHERE n.account_id = ?1 AND n.message_id > r.last_read_id
                    AND m.room_id = r.room_id AND m.deleted_at IS NULL AND m.author_id <> ?1)
             FROM r ORDER BY r.k",
        )
        .bind(user_id)
        .bind(json_ids(room_ids.iter().copied()))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(room_id, last_read_id, unread, mentions)| Unread {
                room_id,
                last_read_id,
                unread,
                mentions,
            })
            .collect())
    }

    /// User ids for the given usernames; unknown names are skipped.
    pub async fn account_ids_by_username(&self, names: &[String]) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM accounts WHERE username IN (SELECT value FROM json_each(?1))
             ORDER BY username",
        )
        .bind(json_strs(names))
        .fetch_all(&self.pool)
        .await?)
    }
}
