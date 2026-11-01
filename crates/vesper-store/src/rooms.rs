use std::collections::HashMap;

use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::RoomInfo;

use crate::{Result, Store, StoreError, constraint};

#[derive(FromRow)]
struct RoomRow {
    id: i64,
    name: String,
    position: i64,
    restricted: bool,
}

impl RoomRow {
    fn into_info(self, members: Vec<String>) -> RoomInfo {
        RoomInfo {
            id: self.id,
            name: self.name,
            position: self.position,
            restricted: self.restricted,
            // The list is kept while a room is open, but only means something when restricted.
            members: if self.restricted { members } else { Vec::new() },
        }
    }
}

const DUPLICATE: &str = "a room with that name exists";

async fn room_by_id(c: &mut SqliteConnection, id: i64) -> Result<RoomInfo> {
    let row = sqlx::query_as::<_, RoomRow>(
        "SELECT id, name, position, restricted FROM rooms WHERE id = ?1",
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
    Ok(row.into_info(members))
}

impl Store {
    /// Every room, by position.
    pub async fn rooms(&self) -> Result<Vec<RoomInfo>> {
        let mut c = self.pool.acquire().await?;
        let rows = sqlx::query_as::<_, RoomRow>(
            "SELECT id, name, position, restricted FROM rooms ORDER BY position, id",
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
        Ok(rows
            .into_iter()
            .map(|r| {
                let m = members.remove(&r.id).unwrap_or_default();
                r.into_info(m)
            })
            .collect())
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

    pub async fn patch_room(
        &self,
        id: i64,
        name: Option<&str>,
        restricted: Option<bool>,
    ) -> Result<RoomInfo> {
        let mut c = self.pool.acquire().await?;
        let n = sqlx::query(
            "UPDATE rooms SET name = coalesce(?2, name), restricted = coalesce(?3, restricted)
             WHERE id = ?1",
        )
        .bind(id)
        .bind(name)
        .bind(restricted)
        .execute(&mut *c)
        .await
        .map_err(|e| constraint(e, DUPLICATE))?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        room_by_id(&mut c, id).await
    }

    /// `ids` must hold every room id exactly once, else `Conflict` and nothing changes.
    pub async fn reorder_rooms(&self, ids: &[i64]) -> Result<()> {
        // IMMEDIATE so a room created meanwhile can not slip between the check and the update.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let all: Vec<i64> = sqlx::query_scalar("SELECT id FROM rooms ORDER BY id")
            .fetch_all(&mut *tx)
            .await?;
        let mut given = ids.to_vec();
        given.sort_unstable();
        if given != all {
            return Err(StoreError::Conflict("the order must list every room once"));
        }
        for (pos, id) in ids.iter().enumerate() {
            sqlx::query("UPDATE rooms SET position = ?2 WHERE id = ?1")
                .bind(id)
                .bind(pos as i64)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Replaces the member list. An unknown room or account is `NotFound` and nothing changes.
    pub async fn set_room_members(&self, id: i64, user_ids: &[String]) -> Result<RoomInfo> {
        let mut tx = self.pool.begin().await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM rooms WHERE id = ?1)")
            .bind(id)
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
}
