#![forbid(unsafe_code)]

pub mod api;
pub use api::*;
pub mod passkeys;
pub use passkeys::*;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_MSG_BYTES: usize = 64 * 1024;

/// WebSocket close code: the session is over (logout, revoked device, password change, ban).
/// The client should go back to the login page instead of reconnecting.
pub const CLOSE_SESSION_ENDED: u16 = 4001;
/// WebSocket close code: a moderator disconnected this user.
pub const CLOSE_DISCONNECTED: u16 = 4003;

/// A running screen share: the frame rate and video bitrate its sender chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ShareInfo {
    pub fps: u8,
    pub bitrate_kbps: u32,
}

/// Someone in a voice room. `user_id` is the account id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Peer {
    pub user_id: String,
    pub name: String,
    #[ts(type = "number")]
    pub room_id: i64,
    pub muted: bool,
    pub deafened: bool,
    pub force_muted: bool,
    /// Set while this person shares their screen.
    #[serde(default)]
    pub share: Option<ShareInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    RoomFull,
    BadMessage,
    Forbidden,
    RateLimited,
    /// A screen share would take the room's shares over the server's total.
    ShareCap,
    /// The server failed (a store error). The cause is in the server log.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ClientMsg {
    Join {
        #[ts(type = "number")]
        room_id: i64,
        offer: String,
    },
    Answer {
        sdp: String,
    },
    SetVoiceState {
        muted: bool,
        deafened: bool,
    },
    /// Start sharing the screen, or change the settings of a share that runs.
    ShareStart {
        fps: u8,
        bitrate_kbps: u32,
    },
    ShareStop,
    /// Start or stop receiving `user_id`'s screen share.
    Watch {
        user_id: String,
        on: bool,
    },
    /// Point at a spot on `user_id`'s shared screen. `x` and `y` run from 0 to 1.
    Point {
        user_id: String,
        x: f32,
        y: f32,
    },
    Leave,
    /// Connectivity check: a voice session that is not shown to anyone.
    Check {
        offer: String,
    },
    Typing {
        #[ts(type = "number")]
        room_id: i64,
    },
    /// Silence the ring of a DM call for this user. The caller stays in the call.
    DeclineRing {
        #[ts(type = "number")]
        room_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerMsg {
    /// The first message on every socket. `rooms` and `voice` hold only what this user can see.
    Hello {
        me: Me,
        users: Vec<UserInfo>,
        rooms: Vec<RoomInfo>,
        voice: Vec<Peer>,
        unread: Vec<Unread>,
    },
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
    PeerState {
        user_id: String,
        muted: bool,
        deafened: bool,
        force_muted: bool,
    },
    /// Someone started, changed or stopped (`None`) a screen share.
    PeerShare {
        user_id: String,
        share: Option<ShareInfo>,
    },
    /// To a sharer: how many people watch the share.
    Watchers {
        count: u32,
    },
    /// Someone pointed at `user_id`'s shared screen.
    Point {
        user_id: String,
        from: String,
        x: f32,
        y: f32,
    },
    /// The rooms this user can see, after any change.
    RoomsChanged {
        rooms: Vec<RoomInfo>,
    },
    UserUpdated {
        user: UserInfo,
    },
    /// Someone signed up. Sent to every socket: every member sees the user list.
    UserCreated {
        user: UserInfo,
    },
    ForceMuted {
        muted: bool,
    },
    /// To a sharer, after its own `PeerShare` with no share: a moderator stopped the share.
    ShareStopped,
    /// A new message in a room this user can see.
    MessageNew {
        message: Message,
    },
    /// An edit, delete, reaction or pin change: the whole message.
    MessageUpdated {
        message: Message,
    },
    /// Someone is typing in a room. There is no stop event: clients drop it after 5 s.
    Typing {
        #[ts(type = "number")]
        room_id: i64,
        user_id: String,
    },
    /// This user's read marker moved (sent to all of their sockets).
    Read {
        #[ts(type = "number")]
        room_id: i64,
        #[ts(type = "number")]
        last_read_id: i64,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    /// Someone started voice in a DM with this user. Sent to this user's open sockets only.
    RingStart {
        #[ts(type = "number")]
        room_id: i64,
        /// The caller's account id.
        user_id: String,
    },
    /// The ring is over: the caller left, someone joined or declined, or it timed out.
    RingStop {
        #[ts(type = "number")]
        room_id: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_roundtrips_json() {
        let m = ClientMsg::Join {
            room_id: 1,
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
