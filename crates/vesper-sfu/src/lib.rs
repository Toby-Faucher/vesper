#![forbid(unsafe_code)]
//! Audio-only SFU on str0m: one UDP socket, rooms keyed by name, Opus forwarded as-is
//! (no decoding, mixing or transcoding).
//!
//! The str0m loop runs on its own OS thread; [`SfuHandle`] is a cloneable async facade over it.
//! Signaling is the caller's job: the client's offer goes into [`SfuHandle::join`], the SFU's
//! renegotiation offers come out as [`SfuEvent::Offer`] and their answers go back through
//! [`SfuHandle::answer`].

mod client;
mod room;
#[cfg(feature = "test-support")]
pub mod testing;

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;

use tokio::sync::{mpsc as tmpsc, oneshot};

pub struct SfuConfig {
    /// Address the single media UDP socket binds to.
    pub bind: SocketAddr,
    /// Addresses clients can reach that socket on; each becomes a host ICE candidate.
    pub advertised: Vec<SocketAddr>,
}

pub struct Sfu;

impl Sfu {
    /// Bind the UDP socket and start the SFU loop on its own thread.
    /// The loop stops once every [`SfuHandle`] clone is dropped.
    pub fn spawn(cfg: SfuConfig) -> std::io::Result<SfuHandle> {
        let socket = UdpSocket::bind(cfg.bind)?;
        let local_addr = socket.local_addr()?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("vesper-sfu".into())
            .spawn(move || room::run(socket, rx, cfg.advertised))?;
        Ok(SfuHandle { tx, local_addr })
    }
}

#[derive(Clone)]
pub struct SfuHandle {
    tx: mpsc::Sender<Command>,
    local_addr: SocketAddr,
}

#[derive(Debug)]
pub struct JoinOk {
    pub answer_sdp: String,
    pub events: tmpsc::UnboundedReceiver<SfuEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SfuEvent {
    /// SFU-initiated renegotiation (tracks added or stopped); answer via [`SfuHandle::answer`].
    Offer(String),
    /// The session ended: ICE timeout, error, [`SfuHandle::leave`], or a new join by the same user.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinError {
    BadOffer(String),
    Shutdown,
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::BadOffer(e) => write!(f, "bad sdp offer: {e}"),
            JoinError::Shutdown => f.write_str("sfu has shut down"),
        }
    }
}

impl std::error::Error for JoinError {}

pub(crate) enum Command {
    Join {
        room: String,
        user: String,
        offer: String,
        reply: oneshot::Sender<Result<JoinOk, JoinError>>,
    },
    Answer {
        user: String,
        sdp: String,
    },
    Leave {
        user: String,
    },
    SetAdvertised(Vec<SocketAddr>),
}

impl SfuHandle {
    /// Start a session for `user` in `room` from the client's SDP offer. A second join by the
    /// same user replaces (and closes) the first session.
    pub async fn join(
        &self,
        room: String,
        user: String,
        offer_sdp: String,
    ) -> Result<JoinOk, JoinError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Join {
            room,
            user,
            offer: offer_sdp,
            reply,
        });
        rx.await.unwrap_or(Err(JoinError::Shutdown))
    }

    /// Answer to the latest [`SfuEvent::Offer`] sent to `user`. A malformed answer closes the session.
    pub fn answer(&self, user: &str, answer_sdp: String) {
        self.send(Command::Answer {
            user: user.into(),
            sdp: answer_sdp,
        });
    }

    /// End `user`'s session; the other room members are offered SDP without its track.
    pub fn leave(&self, user: &str) {
        self.send(Command::Leave { user: user.into() });
    }

    /// Replace the advertised candidate addresses. Applies to sessions created by later
    /// [`join`](Self::join) calls; existing sessions keep the candidates they were given.
    pub fn set_advertised(&self, addrs: Vec<SocketAddr>) {
        self.send(Command::SetAdvertised(addrs));
    }

    /// The address the media socket is actually bound to (useful when `bind` used port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn send(&self, cmd: Command) {
        // A dead loop surfaces as JoinError::Shutdown on join; fire-and-forget calls just no-op.
        let _ = self.tx.send(cmd);
    }
}
