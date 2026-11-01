//! Upload rows. The files live on disk under the data dir; the server owns them. `mime` is the
//! type the server sniffed, never the client's claim.

use sqlx::FromRow;

use crate::{Result, Store, StoreError, constraint};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct UploadRow {
    pub id: String,
    pub room_id: i64,
    pub uploader_id: String,
    pub message_id: Option<i64>,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub created_at: i64,
}

impl Store {
    /// An unknown room or uploader is `NotFound`; a used id is `Conflict`.
    pub async fn insert_upload(&self, u: &UploadRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO uploads (id, room_id, uploader_id, message_id, name, mime, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&u.id)
        .bind(u.room_id)
        .bind(&u.uploader_id)
        .bind(u.message_id)
        .bind(&u.name)
        .bind(&u.mime)
        .bind(u.size)
        .bind(u.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| constraint(e, "upload exists"))?;
        Ok(())
    }

    pub async fn upload(&self, id: &str) -> Result<UploadRow> {
        sqlx::query_as("SELECT * FROM uploads WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)
    }

    /// Sum of all upload sizes.
    pub async fn upload_bytes(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM uploads")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Unattached uploads created before `before`.
    pub async fn stale_uploads(&self, before: i64) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM uploads WHERE message_id IS NULL AND created_at < ?1 ORDER BY id",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Deletes the rows of `ids` that are still unattached and returns those ids, so an upload
    /// attached after `stale_uploads` ran is kept (and its file with it).
    pub async fn delete_uploads(&self, ids: &[String]) -> Result<Vec<String>> {
        let ids = serde_json::to_string(ids).expect("strings serialise");
        Ok(sqlx::query_scalar(
            "DELETE FROM uploads
             WHERE id IN (SELECT value FROM json_each(?1)) AND message_id IS NULL RETURNING id",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?)
    }
}
