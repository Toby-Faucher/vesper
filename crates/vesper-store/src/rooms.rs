use std::collections::HashMap;

use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::{RoomInfo, RoomKind};

use crate::{Result, Store, StoreError, constraint};

#[derive(FromRow)]
struct RoomRow {
    id: i64,
    name: String,
    position: i64,
    restricted: bool,
    archived: bool,
    kind: String,
}

const fn kind_sql(k: RoomKind) -> &'static str {
    match k {
        RoomKind::Room => "room",
        RoomKind::Dm => "dm",
    }
}

fn parse_kind(s: &str) -> Result<RoomKind> {
    match s {
        "room" => Ok(RoomKind::Room),
        "dm" => Ok(RoomKind::Dm),
        _ => Err(StoreError::Db(sqlx::Error::Decode(
            format!("unknown room kind {s:?}").into(),
        ))),
    }
}

impl RoomRow {
    fn into_info(self, members: Vec<String>) -> Result<RoomInfo> {
        Ok(RoomInfo {
            id: self.id,
            kind: parse_kind(&self.kind)?,
            name: self.name,
            position: self.position,
            restricted: self.restricted,
            // An open room has no list (opening clears it), so this only guards a list set while open.
            members: if self.restricted { members } else { Vec::new() },
            archived: self.archived,
        })
    }
}

const DUPLICATE: &str = "a room with that name exists";

async fn room_by_id(c: &mut SqliteConnection, id: i64) -> Result<RoomInfo> {
    let row = sqlx::query_as::<_, RoomRow>(
        "SELECT id, name, position, restricted, archived_at IS NOT NULL AS archived, kind
         FROM rooms WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(StoreError::NotFound)?;
    let members = sqlx::query_scalar(
        "SELECT account_id FROM room_members WHERE room_id = ?1 ORDER BY account_id",
    )
    .bind(id)
    .fetch_all(&mut *c)
    .await?;
    row.into_info(members)
}

impl Store {
    /// Every room, by position.
    pub async fn rooms(&self) -> Result<Vec<RoomInfo>> {
        let mut c = self.pool.acquire().await?;
        let rows = sqlx::query_as::<_, RoomRow>(
            "SELECT id, name, position, restricted, archived_at IS NOT NULL AS archived, kind
             FROM rooms ORDER BY position, id",
        )
        .fetch_all(&mut *c)
        .await?;
        let mut members: HashMap<i64, Vec<String>> = HashMap::new();
        let pairs: Vec<(i64, String)> = sqlx::query_as(
            "SELECT room_id, account_id FROM room_members ORDER BY room_id, account_id",
        )
        .fetch_all(&mut *c)
        .await?;
        for (room, account) in pairs {
            members.entry(room).or_default().push(account);
        }
        rows.into_iter()
            .map(|r| {
                let m = members.remove(&r.id).unwrap_or_default();
                r.into_info(m)
            })
            .collect()
    }

    /// A new open room at the end of the list. A name that differs only in case is `Conflict`.
    pub async fn create_room(&self, name: &str, now: i64) -> Result<RoomInfo> {
        let mut c = self.pool.acquire().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO rooms (name, position, created_at)
             SELECT ?1, coalesce(max(position) + 1, 0), ?2 FROM rooms
             RETURNING id",
        )
        .bind(name)
        .bind(now)
        .fetch_one(&mut *c)
        .await
        .map_err(|e| constraint(e, DUPLICATE))?;
        room_by_id(&mut c, id).await
    }

    /// `archived: Some(true)` on the last room that is not archived is `Conflict`; archiving an
    /// archived room keeps its first time.
    pub async fn patch_room(
        &self,
        id: i64,
        name: Option<&str>,
        restricted: Option<bool>,
        archived: Option<bool>,
        now: i64,
    ) -> Result<RoomInfo> {
        // IMMEDIATE: the update and the list clear commit together, and two archives can not
        // both pass the last-room check.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if archived == Some(true) {
            let others: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM rooms WHERE id != ?1 AND archived_at IS NULL AND kind = ?2",
            )
            .bind(id)
            .bind(kind_sql(RoomKind::Room))
            .fetch_one(&mut *tx)
            .await?;
            if others == 0 {
                return Err(StoreError::Conflict("the last room cannot be archived"));
            }
        }
        let n = sqlx::query(
            "UPDATE rooms SET name = coalesce(?2, name), restricted = coalesce(?3, restricted),
                archived_at = CASE ?4 WHEN 1 THEN coalesce(archived_at, ?5) WHEN 0 THEN NULL
                    ELSE archived_at END
             WHERE id = ?1 AND kind = ?6",
        )
        .bind(id)
        .bind(name)
        .bind(restricted)
        .bind(archived)
        .bind(now)
        .bind(kind_sql(RoomKind::Room))
        .execute(&mut *tx)
        .await
        .map_err(|e| constraint(e, DUPLICATE))?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        // Fail closed: opening a room forgets its list, so closing it again grants nobody.
        if restricted == Some(false) {
            sqlx::query("DELETE FROM room_members WHERE room_id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let room = room_by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(room)
    }

    /// `ids` must hold every room id exactly once, else `Conflict` and nothing changes.
    pub async fn reorder_rooms(&self, ids: &[i64]) -> Result<()> {
        // IMMEDIATE so a room created meanwhile can not slip between the check and the update.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let all: Vec<i64> = sqlx::query_scalar("SELECT id FROM rooms WHERE kind = ?1 ORDER BY id")
            .bind(kind_sql(RoomKind::Room))
            .fetch_all(&mut *tx)
            .await?;
        let mut given = ids.to_vec();
        given.sort_unstable();
        if given != all {
            return Err(StoreError::Conflict("the order must list every room once"));
        }
        for (pos, id) in ids.iter().enumerate() {
            sqlx::query("UPDATE rooms SET position = ?2 WHERE id = ?1 AND kind = ?3")
                .bind(id)
                .bind(pos as i64)
                .bind(kind_sql(RoomKind::Room))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Replaces the member list. An unknown room or account is `NotFound` and nothing changes.
    pub async fn set_room_members(&self, id: i64, user_ids: &[String]) -> Result<RoomInfo> {
        // IMMEDIATE: a deferred read-then-write can fail with SQLITE_BUSY_SNAPSHOT under a racing writer.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM rooms WHERE id = ?1 AND kind = ?2)")
                .bind(id)
                .bind(kind_sql(RoomKind::Room))
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        sqlx::query("DELETE FROM room_members WHERE room_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for user in user_ids {
            sqlx::query("INSERT OR IGNORE INTO room_members (room_id, account_id) VALUES (?1, ?2)")
                .bind(id)
                .bind(user)
                .execute(&mut *tx)
                .await
                .map_err(|e| constraint(e, "duplicate member"))?;
        }
        let room = room_by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(room)
    }

    /// The DM between two different accounts, made if it does not exist; the flag is true when
    /// this call made it. The pair is the same whichever way round it is given. `NotFound` if
    /// either account does not exist.
    pub async fn dm_with(&self, a: &str, b: &str, now: i64) -> Result<(RoomInfo, bool)> {
        if a == b {
            return Err(StoreError::Conflict("a DM needs two different people"));
        }
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let name = format!("dm-{lo}-{hi}");
        // IMMEDIATE: two first messages at once make one room, not two.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let found: Option<i64> =
            sqlx::query_scalar("SELECT id FROM rooms WHERE name = ?1 AND kind = ?2")
                .bind(&name)
                .bind(kind_sql(RoomKind::Dm))
                .fetch_optional(&mut *tx)
                .await?;
        let created = found.is_none();
        let id = match found {
            Some(id) => id,
            None => {
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO rooms (name, position, restricted, kind, created_at)
                     VALUES (?1, 0, 1, ?3, ?2) RETURNING id",
                )
                .bind(&name)
                .bind(now)
                .bind(kind_sql(RoomKind::Dm))
                .fetch_one(&mut *tx)
                .await?;
                for who in [lo, hi] {
                    // The foreign key makes an unknown account fail here.
                    sqlx::query("INSERT INTO room_members (room_id, account_id) VALUES (?1, ?2)")
                        .bind(id)
                        .bind(who)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| constraint(e, "no such account"))?;
                }
                id
            }
        };
        let room = room_by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok((room, created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips_and_unknown_is_an_error() {
        for k in [RoomKind::Room, RoomKind::Dm] {
            assert_eq!(parse_kind(kind_sql(k)).unwrap(), k);
        }
        assert!(matches!(parse_kind("channel"), Err(StoreError::Db(_))));
    }
}
