//! Passkey (WebAuthn) REST bodies. The `options` and `credential` fields are the WebAuthn JSON the
//! browser API takes and gives (with base64url byte fields); the server checks them, the client
//! only passes them through, so they are typed `unknown` on the web side.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One registered passkey of the signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub last_used_at: Option<i64>,
}

/// `GET /api/passkeys`. `enabled` is false when the origin cannot be a WebAuthn relying party
/// (an IP address host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyList {
    pub enabled: bool,
    pub passkeys: Vec<PasskeyInfo>,
}

/// `POST /api/passkeys/register/start` and `/api/passkeys/login/start` answer: the options for
/// `navigator.credentials.create` or `.get`, and the one-use id of the server-side challenge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyChallenge {
    pub challenge_id: String,
    #[ts(type = "unknown")]
    pub options: serde_json::Value,
}

/// `POST /api/passkeys/register/finish` and `/api/passkeys/login/finish`: the browser's answer.
/// `name` is used by register only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyFinish {
    pub challenge_id: String,
    #[ts(type = "unknown")]
    pub credential: serde_json::Value,
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
}

/// `PATCH /api/passkeys/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PasskeyRename {
    pub name: String,
}
