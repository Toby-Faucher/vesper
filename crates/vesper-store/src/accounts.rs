use sqlx::{FromRow, SqliteConnection};
use vesper_protocol::{Role, UserInfo};

use crate::sessions::{NewSession, insert_session};
use crate::{Result, Store, StoreError, constraint, now_ms, parse_role, random_hex, role_str};

pub struct NewAccount<'a> {
    pub username: &'a str,
    pub display_name: &'a str,
    pub password_phc: Option<&'a str>,
    pub role: Role,
}

/// What login needs. Never leaves the server.
pub struct LoginRow {
    pub user_id: String,
    pub password_phc: Option<String>,
    pub banned: bool,
}

// A macro, not a const: sqlx 0.9 takes only `&'static str` SQL, and `concat!` keeps it static.
macro_rules! user_cols {
    () => {
        "a.id, a.username, a.display_name, a.accent, a.role, a.banned_at"
    };
}
pub(crate) use user_cols;

#[derive(FromRow)]
pub(crate) struct AccountRow {
    id: String,
    username: String,
    display_name: String,
    accent: String,
    role: String,
    banned_at: Option<i64>,
}

impl AccountRow {
    pub(crate) fn into_info(self) -> Result<UserInfo> {
        Ok(UserInfo {
            role: parse_role(&self.role)?,
            user_id: self.id,
            username: self.username,
            display_name: self.display_name,
            accent: self.accent,
            banned: self.banned_at.is_some(),
        })
    }
}

async fn insert_account(
    c: &mut SqliteConnection,
    a: &NewAccount<'_>,
    role: Role,
) -> Result<String> {
    let id = random_hex(16);
    sqlx::query(
        "INSERT INTO accounts (id, username, display_name, password_phc, role, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(a.username)
    .bind(a.display_name)
    .bind(a.password_phc)
    .bind(role_str(role))
    .bind(now_ms())
    .execute(c)
    .await
    .map_err(|e| constraint(e, "username taken"))?;
    Ok(id)
}

pub(crate) async fn user_by_id(c: &mut SqliteConnection, id: &str) -> Result<UserInfo> {
    sqlx::query_as::<_, AccountRow>(concat!(
        "SELECT ",
        user_cols!(),
        " FROM accounts a WHERE a.id = ?1"
    ))
    .bind(id)
    .fetch_optional(c)
    .await?
    .ok_or(StoreError::NotFound)?
    .into_info()
}

impl Store {
    pub async fn create_account(&self, a: NewAccount<'_>) -> Result<UserInfo> {
        let mut c = self.pool.acquire().await?;
        let id = insert_account(&mut c, &a, a.role).await?;
        user_by_id(&mut c, &id).await
    }

    /// Uses the invite, takes its role (`a.role` is ignored), creates the account and its first
    /// session, all in one transaction. Returns the user and the session id. An owner invite is
    /// `InviteInvalid` once an owner exists.
    pub async fn signup(
        &self,
        invite_hash: &[u8; 32],
        a: NewAccount<'_>,
        s: NewSession<'_>,
    ) -> Result<(UserInfo, i64)> {
        // IMMEDIATE takes the write lock up front, so racing signups queue on busy_timeout.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let role: Option<String> = sqlx::query_scalar(
            "UPDATE invites SET uses = uses + 1
             WHERE code_hash = ?1 AND uses < max_uses AND expires_at > ?2 AND revoked_at IS NULL
             RETURNING role",
        )
        .bind(&invite_hash[..])
        .bind(s.now)
        .fetch_optional(&mut *tx)
        .await?;
        let role = parse_role(&role.ok_or(StoreError::InviteInvalid)?)?;
        // Two old create-owner links must not make two owners. Dropping `tx` undoes the use.
        if role == Role::Owner && owner_exists(&mut tx).await? {
            return Err(StoreError::InviteInvalid);
        }
        let id = insert_account(&mut tx, &a, role).await?;
        let sid = insert_session(&mut tx, &id, &s).await?;
        let user = user_by_id(&mut tx, &id).await?;
        tx.commit().await?;
        Ok((user, sid))
    }

    pub async fn login_row(&self, username: &str) -> Result<Option<LoginRow>> {
        let row: Option<(String, Option<String>, Option<i64>)> =
            sqlx::query_as("SELECT id, password_phc, banned_at FROM accounts WHERE username = ?1")
                .bind(username)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(user_id, password_phc, banned_at)| LoginRow {
            user_id,
            password_phc,
            banned: banned_at.is_some(),
        }))
    }

    pub async fn user(&self, id: &str) -> Result<UserInfo> {
        user_by_id(&mut *self.pool.acquire().await?, id).await
    }

    pub async fn users(&self) -> Result<Vec<UserInfo>> {
        sqlx::query_as::<_, AccountRow>(concat!(
            "SELECT ",
            user_cols!(),
            " FROM accounts a ORDER BY a.username"
        ))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(AccountRow::into_info)
        .collect()
    }

    pub async fn update_profile(
        &self,
        id: &str,
        display_name: Option<&str>,
        accent: Option<&str>,
    ) -> Result<UserInfo> {
        let mut c = self.pool.acquire().await?;
        let n = sqlx::query(
            "UPDATE accounts SET display_name = coalesce(?2, display_name), accent = coalesce(?3, accent)
             WHERE id = ?1",
        )
        .bind(id)
        .bind(display_name)
        .bind(accent)
        .execute(&mut *c)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        user_by_id(&mut c, id).await
    }

    /// Sets the password hash and deletes every other session of the user.
    pub async fn set_password(&self, id: &str, phc: &str, keep_session: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let n = sqlx::query("UPDATE accounts SET password_phc = ?2 WHERE id = ?1")
            .bind(id)
            .bind(phc)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        sqlx::query("DELETE FROM sessions WHERE account_id = ?1 AND id != ?2")
            .bind(id)
            .bind(keep_session)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_role(&self, id: &str, role: Role) -> Result<UserInfo> {
        let mut c = self.pool.acquire().await?;
        let n = sqlx::query("UPDATE accounts SET role = ?2 WHERE id = ?1")
            .bind(id)
            .bind(role_str(role))
            .execute(&mut *c)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        user_by_id(&mut c, id).await
    }

    /// A ban deletes all of the user's sessions. Banning twice keeps the first time.
    pub async fn set_banned(&self, id: &str, banned: bool, now: i64) -> Result<UserInfo> {
        let mut tx = self.pool.begin().await?;
        let n = sqlx::query(
            "UPDATE accounts SET banned_at = CASE WHEN ?2 THEN coalesce(banned_at, ?3) END
             WHERE id = ?1",
        )
        .bind(id)
        .bind(banned)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        if banned {
            sqlx::query("DELETE FROM sessions WHERE account_id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let user = user_by_id(&mut tx, id).await?;
        tx.commit().await?;
        Ok(user)
    }

    pub async fn owner_exists(&self) -> Result<bool> {
        owner_exists(&mut *self.pool.acquire().await?).await
    }
}

async fn owner_exists(c: &mut SqliteConnection) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM accounts WHERE role = 'owner')")
            .fetch_one(c)
            .await?,
    )
}
