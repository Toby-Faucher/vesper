use sqlx::FromRow;
use vesper_protocol::{InviteInfo, Role};

use crate::{Result, Store, StoreError, constraint, now_ms, parse_role, role_str};

macro_rules! invite_cols {
    () => {
        "id, role, uses, max_uses, expires_at, created_by, revoked_at"
    };
}

#[derive(FromRow)]
struct InviteRow {
    id: i64,
    role: String,
    uses: u32,
    max_uses: u32,
    expires_at: i64,
    created_by: Option<String>,
    revoked_at: Option<i64>,
}

impl InviteRow {
    fn into_info(self) -> Result<InviteInfo> {
        Ok(InviteInfo {
            id: self.id,
            role: parse_role(&self.role)?,
            uses: self.uses,
            max_uses: self.max_uses,
            expires_at: self.expires_at,
            created_by: self.created_by,
            revoked: self.revoked_at.is_some(),
        })
    }
}

impl Store {
    pub async fn create_invite(
        &self,
        code_hash: &[u8; 32],
        role: Role,
        max_uses: u32,
        expires_at: i64,
        created_by: Option<&str>,
    ) -> Result<InviteInfo> {
        sqlx::query_as::<_, InviteRow>(concat!(
            "INSERT INTO invites (code_hash, role, max_uses, expires_at, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING ",
            invite_cols!()
        ))
        .bind(&code_hash[..])
        .bind(role_str(role))
        .bind(max_uses)
        .bind(expires_at)
        .bind(created_by)
        .bind(now_ms())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| constraint(e, "invite exists"))?
        .into_info()
    }

    /// Every invite, newest first.
    pub async fn invites(&self) -> Result<Vec<InviteInfo>> {
        sqlx::query_as::<_, InviteRow>(concat!(
            "SELECT ",
            invite_cols!(),
            " FROM invites ORDER BY id DESC"
        ))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(InviteRow::into_info)
        .collect()
    }

    /// Revoking twice keeps the first time.
    pub async fn revoke_invite(&self, id: i64, now: i64) -> Result<()> {
        let n =
            sqlx::query("UPDATE invites SET revoked_at = coalesce(revoked_at, ?2) WHERE id = ?1")
                .bind(id)
                .bind(now)
                .execute(&self.pool)
                .await?
                .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// The role a usable invite grants; None if unknown, used up, expired or revoked.
    pub async fn invite_role(&self, code_hash: &[u8; 32], now: i64) -> Result<Option<Role>> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM invites
             WHERE code_hash = ?1 AND uses < max_uses AND expires_at > ?2 AND revoked_at IS NULL",
        )
        .bind(&code_hash[..])
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        role.as_deref().map(parse_role).transpose()
    }
}
