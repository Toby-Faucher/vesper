//! The SFU loop: owns the UDP socket and every client, routes packets, and forwards media
//! between clients of the same room. Adapted from str0m's `examples/chat.rs` `run`.

use std::collections::{HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use str0m::change::SdpOffer;
use str0m::media::{KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Input, Rtc};
use tokio::sync::mpsc as tmpsc;
use tracing::{debug, warn};

use crate::client::{Client, ClientId, Propagated};
use crate::{Command, JoinError, JoinOk, SfuConfig, SfuEvent};

/// Upper bound on one loop iteration's socket wait, so commands (joins, answers) are picked up
/// within this time even when the socket is idle.
const MAX_WAIT: Duration = Duration::from_millis(10);

pub(crate) fn run(socket: UdpSocket, rx: Receiver<Command>, cfg: SfuConfig) {
    let socket_addr = socket.local_addr().expect("bound socket has an address");
    let mut advertised = cfg.advertised.clone();
    let mut clients: Vec<Client> = vec![];
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    // Force-muted users: their audio is dropped at the forward point, whatever the client sends.
    let mut muted: HashSet<String> = HashSet::new();
    let mut buf = vec![0; 2000];

    loop {
        // Dropping a client drops its incoming tracks, which makes the other clients stop them.
        clients.retain_mut(|c| {
            let alive = c.rtc.is_alive();
            if !alive {
                c.close();
            }
            alive
        });

        loop {
            match rx.try_recv() {
                Ok(cmd) => handle_command(
                    cmd,
                    &mut clients,
                    &mut muted,
                    &mut advertised,
                    socket_addr,
                    &cfg,
                ),
                Err(TryRecvError::Empty) => break,
                // Every SfuHandle is gone: shut down, telling clients their sessions ended.
                Err(TryRecvError::Disconnected) => {
                    clients.iter_mut().for_each(Client::close);
                    return;
                }
            }
        }

        let mut timeout = Instant::now() + MAX_WAIT;
        for client in clients.iter_mut() {
            let t = poll_until_timeout(client, &mut to_propagate, &socket);
            timeout = timeout.min(t);
        }

        if let Some(p) = to_propagate.pop_front() {
            propagate(&p, &mut clients, &muted);
            continue;
        }

        // The read timeout must not be 0.
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        // set_read_timeout only fails for a zero duration (ruled out above) or a bad fd, which
        // would be our bug, so panicking here is correct; AliveGuard reports the dead thread.
        socket
            .set_read_timeout(Some(wait))
            .expect("non-zero read timeout on an owned socket");

        match socket.recv_from(&mut buf) {
            Ok((n, source)) => route(&buf[..n], source, socket_addr, &mut clients),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // e.g. ECONNREFUSED from an ICMP unreachable after a client vanished.
            Err(e) => debug!("udp recv failed: {e}"),
        }

        let now = Instant::now();
        for client in &mut clients {
            client.handle_input(Input::Timeout(now));
        }
    }
}

fn receive(data: &[u8], source: SocketAddr, destination: SocketAddr) -> Option<Input<'_>> {
    Some(Input::Receive(
        Instant::now(),
        Receive {
            proto: Protocol::Udp,
            source,
            destination,
            contents: data.try_into().ok()?,
        },
    ))
}

/// Warn about advertised sets clients cannot use to reach a socket bound to `bind`.
pub(crate) fn check_advertised(bind: SocketAddr, advertised: &[SocketAddr]) {
    // A [::] socket is dual-stack (on Linux by default), so it serves both families.
    let dual_stack = bind.is_ipv6() && bind.ip().is_unspecified();
    if advertised.is_empty() {
        warn!("no advertised addresses: clients cannot reach the SFU on {bind}");
    } else if !dual_stack && !advertised.iter().any(|a| a.is_ipv4() == bind.is_ipv4()) {
        warn!("no advertised address {advertised:?} has the same IP family as bind {bind}");
    }
}

fn route(data: &[u8], source: SocketAddr, socket_addr: SocketAddr, clients: &mut [Client]) {
    // A dual-stack socket reports IPv4 peers as ::ffff:a.b.c.d; str0m and the SDP know them as
    // plain IPv4, so canonicalize (Client maps them back when sending).
    let source = SocketAddr::new(source.ip().to_canonical(), source.port());
    // rtc.accepts() demultiplexes on source address / STUN credentials, not destination.
    let Some(probe) = receive(data, source, socket_addr) else {
        return;
    };
    let Some(client) = clients.iter_mut().find(|c| c.rtc.accepts(&probe)) else {
        // Common: a client's first STUN can beat its join being processed.
        debug!("no client accepts udp input from {source}");
        return;
    };
    let destination = client.local_addr_for(source, socket_addr);
    if let Some(input) = receive(data, source, destination) {
        client.handle_input(input);
    }
}

fn handle_command(
    cmd: Command,
    clients: &mut Vec<Client>,
    muted: &mut HashSet<String>,
    advertised: &mut Vec<SocketAddr>,
    socket_addr: SocketAddr,
    cfg: &SfuConfig,
) {
    match cmd {
        Command::Join {
            room,
            user,
            offer,
            reply,
        } => {
            let (mut client, answer_sdp, events) =
                match new_client(&room, &user, &offer, advertised, socket_addr, cfg) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        return;
                    }
                };
            if let Some(old) = clients.iter_mut().find(|c| c.user == user) {
                // Superseded: the retain at the top of the loop closes and removes it.
                old.rtc.disconnect();
            }
            // Subscribe the newcomer to everyone already in the room.
            for track in clients
                .iter()
                .filter(|c| c.room == room && c.user != user && c.rtc.is_alive())
                .flat_map(|c| c.tracks_in.iter())
            {
                client.handle_track_open(Arc::downgrade(track));
            }
            if reply.send(Ok(JoinOk { answer_sdp, events })).is_ok() {
                clients.push(client);
            }
        }
        Command::Answer { user, sdp } => match live(clients, &user) {
            Some(c) => c.handle_answer(&sdp),
            None => debug!("answer for unknown user {user}"),
        },
        Command::Leave { user } => {
            if let Some(c) = live(clients, &user) {
                c.rtc.disconnect();
            }
        }
        Command::SetMuted { user, muted: true } => {
            muted.insert(user);
        }
        Command::SetMuted { user, muted: false } => {
            muted.remove(&user);
        }
        Command::SetSharing { user, on } => {
            if let Some(c) = live(clients, &user) {
                c.sharing = on;
            }
        }
        Command::Watch { viewer, sharer, on } => {
            // A new viewer needs a keyframe to show anything.
            if on && let Some(s) = live(clients, &sharer) {
                let video = s.tracks_in.iter().find(|t| t.kind == MediaKind::Video);
                if let Some(mid) = video.map(|t| t.mid) {
                    s.request_keyframe(mid, KeyframeRequestKind::Pli);
                }
            }
            if let Some(c) = live(clients, &viewer) {
                if on {
                    c.watching.insert(sharer);
                } else {
                    c.watching.remove(&sharer);
                }
            }
        }
        Command::SetAdvertised(addrs) => {
            check_advertised(socket_addr, &addrs);
            *advertised = addrs;
        }
        #[cfg(feature = "test-support")]
        Command::Panic => panic!("SFU panic requested by a test"),
    }
}

/// The live session of `user`: after a re-join the dead old session may still be in the list.
fn live<'a>(clients: &'a mut [Client], user: &str) -> Option<&'a mut Client> {
    clients
        .iter_mut()
        .find(|c| c.user == user && c.rtc.is_alive())
}

/// Untrusted input: a client needs exactly one audio m-line (its voice) and may have one video
/// m-line (its screen). More would let one client make the SFU hold (and offer others)
/// arbitrarily many tracks. Returns the offer and whether it had the video m-line.
pub(crate) fn parse_offer(offer: &str) -> Result<(SdpOffer, bool), JoinError> {
    let bad = |e: &dyn std::fmt::Display| JoinError::BadOffer(e.to_string());
    let m_lines: Vec<&str> = offer.lines().filter(|l| l.starts_with("m=")).collect();
    let count = |kind: &str| m_lines.iter().filter(|l| l.starts_with(kind)).count();
    let (audio, video) = (count("m=audio "), count("m=video "));
    if audio != 1 || video > 1 || m_lines.len() != audio + video {
        return Err(bad(&format!(
            "expected one audio and at most one video m-line, got {audio} audio and {video} video of {} (first: {:?})",
            m_lines.len(),
            m_lines.first()
        )));
    }
    let offer = SdpOffer::from_sdp_string(offer).map_err(|e| bad(&e))?;
    Ok((offer, video == 1))
}

fn new_client(
    room: &str,
    user: &str,
    offer: &str,
    advertised: &[SocketAddr],
    socket_addr: SocketAddr,
    cfg: &SfuConfig,
) -> Result<(Client, String, tmpsc::UnboundedReceiver<SfuEvent>), JoinError> {
    let bad = |e: &dyn std::fmt::Display| JoinError::BadOffer(e.to_string());
    let (offer, video) = parse_offer(offer)?;
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .enable_vp8(true)
        .build(Instant::now());
    for addr in advertised {
        match Candidate::host(*addr, "udp") {
            Ok(c) => {
                rtc.add_local_candidate(c);
            }
            Err(e) => warn!("skipping advertised address {addr}: {e}"),
        }
    }
    let answer = rtc.sdp_api().accept_offer(offer).map_err(|e| bad(&e))?;
    let (tx, events) = tmpsc::unbounded_channel();
    let mut client = Client::new(
        rtc,
        room.into(),
        user.into(),
        tx,
        advertised.to_vec(),
        socket_addr.is_ipv6(),
        cfg,
    );
    client.video = video;
    Ok((client, answer.to_sdp_string(), events))
}

fn poll_until_timeout(
    client: &mut Client,
    queue: &mut VecDeque<Propagated>,
    socket: &UdpSocket,
) -> Instant {
    loop {
        if !client.rtc.is_alive() {
            return Instant::now();
        }
        match client.poll_output(socket) {
            Propagated::Timeout(t) => return t,
            Propagated::Noop => {}
            p => queue.push_back(p),
        }
    }
}

/// Deliver one client's output to the other clients of its room. Video media goes only from a
/// sharing sender to the clients watching it; a force-mute drops only audio. A keyframe request
/// goes to the sender of the track it is about.
fn propagate(propagated: &Propagated, clients: &mut [Client], muted: &HashSet<String>) {
    let origin = match propagated {
        Propagated::TrackOpen(c, _) | Propagated::MediaData(c, _) => *c,
        Propagated::KeyframeRequest(viewer, mid, kind) => {
            return forward_keyframe_request(clients, *viewer, *mid, *kind);
        }
        Propagated::Noop | Propagated::Timeout(_) => return,
    };
    let Some(sender) = clients.iter().find(|c| c.id == origin) else {
        return;
    };
    let mut video = false;
    if let Propagated::MediaData(_, data) = propagated {
        let Some(track) = sender.tracks_in.iter().find(|t| t.mid == data.mid) else {
            return;
        };
        video = track.kind == MediaKind::Video;
        let dropped = if video {
            !sender.sharing
        } else {
            muted.contains(&sender.user)
        };
        if dropped {
            return;
        }
    }
    let (room, user) = (sender.room.clone(), sender.user.clone());
    for client in clients
        .iter_mut()
        .filter(|c| c.id != origin && c.room == room)
    {
        match propagated {
            Propagated::TrackOpen(_, track) => client.handle_track_open(track.clone()),
            Propagated::MediaData(_, data) => {
                if !video || client.watching.contains(&user) {
                    client.handle_media_data_out(origin, data);
                }
            }
            Propagated::KeyframeRequest(..) | Propagated::Noop | Propagated::Timeout(_) => {}
        }
    }
}

/// Pass `viewer`'s keyframe request on its outgoing m-line `mid` to the client sending that track.
fn forward_keyframe_request(
    clients: &mut [Client],
    viewer: ClientId,
    mid: Mid,
    kind: KeyframeRequestKind,
) {
    let Some(track) = clients
        .iter()
        .find(|c| c.id == viewer)
        .and_then(|c| c.track_in_of(mid))
    else {
        return;
    };
    if let Some(origin) = clients.iter_mut().find(|c| c.id == track.origin) {
        origin.request_keyframe(track.mid, kind);
    }
}
