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
