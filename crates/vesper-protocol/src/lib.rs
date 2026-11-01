#![forbid(unsafe_code)]

pub mod api;
pub use api::*;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_MSG_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Peer {
    pub user_id: String,
    pub name: String,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    BadKey,
    RoomFull,
    BadMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ClientMsg {
    Join {
        room: String,
        name: String,
        key: String,
        offer: String,
    },
    Answer {
        sdp: String,
    },
    SetMuted {
        muted: bool,
    },
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerMsg {
    Joined {
        user_id: String,
        answer: String,
        peers: Vec<Peer>,
    },
    Offer {
        sdp: String,
    },
    PeerJoined {
        peer: Peer,
    },
    PeerLeft {
        user_id: String,
    },
    PeerMuted {
        user_id: String,
        muted: bool,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_roundtrips_json() {
        let m = ClientMsg::Join {
            room: "r".into(),
            name: "n".into(),
            key: "k".into(),
            offer: "o".into(),
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""type":"join""#));
        assert_eq!(serde_json::from_str::<ClientMsg>(&s).unwrap(), m);
    }

    #[test]
    fn unknown_type_is_error() {
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn export_ts_bindings() {
        // explicit export: the generated export_bindings_* tests run in parallel with this one
        ClientMsg::export().unwrap();
        let p =
            std::path::Path::new(&std::env::var("TS_RS_EXPORT_DIR").unwrap()).join("ClientMsg.ts");
        assert!(p.exists(), "{p:?}");
    }
}
