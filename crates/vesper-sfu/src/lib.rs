#![forbid(unsafe_code)]
//! SFU on str0m: one UDP socket, rooms keyed by name, Opus voice and one VP8 screen per client
//! forwarded as-is (no decoding, mixing or transcoding). Video goes only from a client the server
//! marked as sharing ([`SfuHandle::set_sharing`]) to the clients watching it ([`SfuHandle::watch`]).
//!
//! The str0m loop runs on its own OS thread; [`SfuHandle`] is a cloneable async facade over it.
//! Signaling is the caller's job: the client's offer goes into [`SfuHandle::join`], the SFU's
//! renegotiation offers come out as [`SfuEvent::Offer`] and their answers go back through
//! [`SfuHandle::answer`].

mod client;
mod meter;
mod room;
#[cfg(feature = "test-support")]
pub mod testing;

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use tokio::sync::{mpsc as tmpsc, oneshot};

/// Build with struct syntax plus `..Default::default()` for the fields you don't set.
#[derive(Debug, Clone)]
pub struct SfuConfig {
    /// Address the single media UDP socket binds to.
    pub bind: SocketAddr,
    /// Addresses clients can reach that socket on; each becomes a host ICE candidate.
    pub advertised: Vec<SocketAddr>,
    /// How long a client may take to answer an [`SfuEvent::Offer`] before its session is closed.
    pub answer_timeout: Duration,
    /// A share whose incoming video stays above this many bits per second for
    /// `share_ceiling_secs` one-second windows in a row is cut ([`SfuEvent::ShareCut`]).
    pub share_ceiling_bps: u64,
    pub share_ceiling_secs: u32,
}

impl Default for SfuConfig {
    /// Binds `0.0.0.0:0` with no advertised addresses (set them!), a 10 s answer timeout and a
    /// share ceiling of 35 Mbps for 5 s (above the client's 30 Mbps maximum).
    fn default() -> Self {
        SfuConfig {
            bind: (Ipv4Addr::UNSPECIFIED, 0).into(),
            advertised: vec![],
            answer_timeout: Duration::from_secs(10),
            share_ceiling_bps: 35_000_000,
            share_ceiling_secs: 5,
        }
    }
}

pub struct Sfu;

impl Sfu {
    /// Bind the UDP socket and start the SFU loop on its own thread.
    /// The loop stops once every [`SfuHandle`] clone is dropped.
    pub fn spawn(cfg: SfuConfig) -> std::io::Result<SfuHandle> {
        let socket = UdpSocket::bind(cfg.bind)?;
        let local_addr = socket.local_addr()?;
        room::check_advertised(local_addr, &cfg.advertised);
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let guard = AliveGuard(alive.clone());
        std::thread::Builder::new()
            .name("vesper-sfu".into())
            .spawn(move || {
                let _guard = guard; // dropped on return and on panic unwind
                room::run(socket, rx, cfg)
            })?;
        Ok(SfuHandle {
            tx,
            local_addr,
            alive,
        })
    }
}

/// Clears the liveness flag when the SFU thread ends, however it ends.
struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
        if std::thread::panicking() {
            tracing::error!("SFU thread panicked: voice is down until the server restarts");
        }
    }
}

#[derive(Clone)]
pub struct SfuHandle {
    tx: mpsc::Sender<Command>,
    local_addr: SocketAddr,
    alive: Arc<AtomicBool>,
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
    /// The session ended: ICE timeout, error, an offer left unanswered for
    /// [`SfuConfig::answer_timeout`], [`SfuHandle::leave`], or a new join by the same user.
    Closed,
    /// The SFU stopped forwarding this session's share: its video stayed over
    /// [`SfuConfig::share_ceiling_bps`]. Sent once; the session is no longer sharing, as after
    /// [`SfuHandle::set_sharing`] with `false`.
    ShareCut,
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

/// The offer checks [`SfuHandle::join`] makes first, without the SFU: a caller can refuse a bad
/// offer before it ends anything. `join` can still refuse an offer that passes.
pub fn check_offer(offer_sdp: &str) -> Result<(), JoinError> {
    room::parse_offer(offer_sdp).map(drop)
}

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
    SetMuted {
        user: String,
        muted: bool,
    },
    SetSharing {
        user: String,
        on: bool,
    },
    Watch {
        viewer: String,
        sharer: String,
        on: bool,
    },
    SetAdvertised(Vec<SocketAddr>),
    #[cfg(feature = "test-support")]
    Panic,
}

impl SfuHandle {
    /// Start a session for `user` in `room` from the client's SDP offer. A second join by the
    /// same user replaces (and closes) the first session. The offer must carry exactly one audio
    /// m-line and at most one video m-line (no data channel); anything else is
    /// [`JoinError::BadOffer`]. Only a client whose offer had a video m-line is offered others' video.
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
        rx.await.unwrap_or_else(|_| {
            // The loop always replies, so a dropped reply means its thread is dying: say so now,
            // before the unwind reaches the AliveGuard, so callers see is_alive() == false.
            self.alive.store(false, Ordering::Release);
            Err(JoinError::Shutdown)
        })
    }

    /// Answer to the latest [`SfuEvent::Offer`] sent to `user`. A malformed answer closes the
    /// session, and so does no answer within [`SfuConfig::answer_timeout`]: the SFU makes no
    /// further offers to a client while one is outstanding, so it cannot be left waiting forever.
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

    /// Force-mute `user` or lift it: while muted, the SFU forwards none of their audio. Keyed by
    /// user, not session, so it holds across leave and rejoin until lifted here.
    pub fn set_muted(&self, user: &str, muted: bool) {
        self.send(Command::SetMuted {
            user: user.into(),
            muted,
        });
    }

    /// Start or stop forwarding `user`'s video to the clients watching them. Keyed by session:
    /// a new join by `user` starts not sharing.
    pub fn set_sharing(&self, user: &str, on: bool) {
        self.send(Command::SetSharing {
            user: user.into(),
            on,
        });
    }

    /// Start or stop sending `sharer`'s video to `viewer`, while `sharer` is sharing. Keyed by the
    /// viewer's session: a new join by `viewer` starts watching nobody. Starting asks the sharer
    /// for a keyframe, so the viewer has a picture right away.
    pub fn watch(&self, viewer: &str, sharer: &str, on: bool) {
        self.send(Command::Watch {
            viewer: viewer.into(),
            sharer: sharer.into(),
            on,
        });
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

    /// False once the SFU thread has ended (a panic inside str0m, or every handle dropped).
    /// Joins then fail with [`JoinError::Shutdown`].
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Make the SFU thread panic, to test what happens when it dies.
    #[cfg(feature = "test-support")]
    pub fn crash_for_test(&self) {
        self.send(Command::Panic);
    }

    fn send(&self, cmd: Command) {
        // A dead loop surfaces as JoinError::Shutdown on join; fire-and-forget calls just no-op.
        let _ = self.tx.send(cmd);
    }
}
